//! The beacon's node-facing seam: one [`build`] call, one `Arc<dyn Beacon>`, one
//! [`Tasks`].
//!
//! How the epoch key is agreed, where the artifact is stored, how a share is
//! derived and how a peer is served all stay behind it. What crosses it is two
//! task handles and the staking reads that necessarily run the other way — they
//! read the reth state this crate has no access to, and they arrive as ONE
//! [`CommitteeReads`] so every one of them lands on the same cursor.

use alloy_primitives::B256;
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
use commonware_p2p::{Provider, Receiver, Sender};
use commonware_resolver::p2p::{Config as ResolverConfig, Engine as ResolverEngine};
use commonware_runtime::{BufferPooler, Clock, Handle, Metrics, Spawner, Storage};
use commonware_utils::ordered::Set;
use fluentbase_bls::scheme::EpochCommittee;
use fluentbase_bls::{
    beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair, PeerPubkey, ShareSealKey,
};
use fluentbase_p2p::NoopBlocker;
use futures::future::BoxFuture;
use rand_core::CryptoRngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, RwLock},
    time::Duration,
};
use tokio::sync::{mpsc, watch, Notify};
use tracing::{debug, error, info, warn};

use crate::{
    beacon::{
        actor::{
            AgreedOutcomeAt, CeremonyStore, CommitteeFor, DkgActor, DkgLogIndex, PinnedRequest,
            PullArtifact, StoredArtifact, Wiring,
        },
        artifact::{
            self, AcquireArtifact, AcquireMint, ArtifactBridge, ArtifactPull, ArtifactStore,
            ChangedAt, CommitteeSource, KeyIndex, PullAnswer,
        },
        dkg_agree::{AgreedArtifact, ConfirmPool},
        dkg_engine::{
            spawn_agreement_launcher, AgreementMuxes, AgreementPlaneConfig, AgreementTimeouts,
        },
        log_resolver::{BeaconFetchHandler, BeaconFetchKey, LogFetcher, LogHandler, LogMessage},
        metrics::BeaconMetrics,
        outcome::group_public_key,
        share_state::{self, ShareState},
        surface::{LiveBeacon, LiveBeaconConfig},
        Beacon, BeaconEvent,
    },
    dpos::{ARTIFACT_JOURNAL_PARTITION, MINT_MEMO_PARTITION, SEED_JOURNAL_PARTITION},
    outer::SharedMux,
};

/// The resolver mailbox both beacon subjects — the `{epoch, dealer, hash}` dealer log
/// and the epoch-key artifact — ride.
pub(crate) type BeaconResolver = commonware_resolver::p2p::Mailbox<BeaconFetchKey, PeerPubkey>;

/// That mailbox narrowed to the dealer-log key space, which is all the ceremony
/// and the agreement automaton ever see of it.
pub(crate) type BeaconLogs = LogFetcher<BeaconResolver>;

/// Backfill cadence for the beacon resolver, mirroring the marshal resolver's:
/// a dealer-log or artifact fetch is rare (one restarted member's catch-up).
const RESOLVER_INITIAL: Duration = Duration::from_millis(100);
const RESOLVER_TIMEOUT: Duration = Duration::from_secs(5);
const RESOLVER_RETRY: Duration = Duration::from_millis(500);

/// Resolver mailbox depth, matching the plane's other mailboxes.
const RESOLVER_MAILBOX: usize = 256;

/// Depth of the actor's pinned-set question channel. One agreement instance per
/// target epoch asks it, so this is never the bound.
const PINNED_MAILBOX: usize = 256;

/// Mailbox depth of each agreement instance's simplex actors, matching the
/// ordering plane's.
const AGREEMENT_MAILBOX: usize = 256;

/// Depth of the plane's edge channels — the dealing-closed announcement, the
/// instance intake and the write-back's two hops. All of them carry at most one
/// item per epoch boundary, and the announcement is re-sent on every height tick,
/// so a full channel costs a tick and never an event.
const EDGE_MAILBOX: usize = 16;

/// Every staking-state read the beacon needs, behind one trait and one cursor.
///
/// ONE trait rather than the four closures plus a separate `dkgQual` state-hash
/// resolver it replaces, and the reason is the cursor: `committee[E]` was already
/// read at `max(EL-finalized, live)` while `dkgQual[E]` was read at the finalized
/// hash alone, so the two answers could describe different blocks. They are the
/// two halves of ONE question — "is there a re-mint at E, and who is in it" — and
/// the split let a node see a committee it could not yet see the qual bit for
/// (`.dpos-study/DECISIONS.md`, Д-7).
///
/// [`Self::read_at`] is the cursor, resolved ONCE per compound read. That is what
/// makes [`Self::committee_pair`] structurally unable to straddle a block: it is a
/// provided method over two [`Self::committee`] calls at one hash, and a
/// validator that rotates its consensus key between two independent reads can no
/// longer make the node see a committee change the contract did not.
///
/// There is no second cursor for the `dkgQual` leg any more. `qual_read_at`
/// existed for ONE window — a live cert cursor with no EL-finalized marker,
/// where `read_at` fell back to the GENESIS hash and the write-once bit cache
/// (then `carry::frozen_dkg_qual`, now [`super::artifact::MintIndex`]'s) would have
/// frozen `false` for that epoch for the life of the process. The single implementation of this trait is now
/// [`crate::committee::CommitteeReadsFacade`], whose anchor is
/// `executed_state_hash(ordering_finalized)` and which has no genesis fallback
/// at all: below `commit_height(E)` the module answers "not readable" without
/// reading anything, and at or above it the bit is final, because the contract
/// writes it in the same `commit_epoch_committee` call that writes the
/// committee (`contracts/staking/src/consensus.rs:632-635`).
pub trait CommitteeReads: Send + Sync {
    /// The state hash every read below is taken at — including the `dkgQual`
    /// leg — or `None` where this node cannot read state yet.
    fn read_at(&self) -> Option<B256>;

    /// `committee[epoch]` as the ordered peer set at `at` — the ceremony roster
    /// and the AM5 idx→pubkey mapping.
    fn committee(&self, epoch: u64, at: B256) -> Option<Set<PeerPubkey>>;

    /// The SAME frozen committee with its BLS half, projected into the participant
    /// BiMap a certificate is verified under.
    fn committee_bls(&self, epoch: u64, at: B256) -> Option<EpochCommittee>;

    /// One raw on-chain `(dkgQual[epoch], committee[epoch] is committed)` read at
    /// `at`. The freeze/memo rule that turns it into the carry-forward arbiter stays
    /// on this side of the boundary: [`changed_bit`] drops the
    /// `committed` leg (Д-7 — the facade answers it unconditionally `true`) and
    /// [`super::artifact::MintIndex`] caches the decided bit and walks to the mint.
    fn dkg_qual(&self, epoch: u64, at: B256) -> Option<(bool, bool)>;

    /// `committee[target−1]` and `committee[target]` at ONE state hash — the
    /// ceremony-start decision's input. Provided, and deliberately not
    /// overridable-by-accident: the single `read_at` above is the whole point.
    fn committee_pair(&self, target: u64) -> Option<(Set<PeerPubkey>, Set<PeerPubkey>)> {
        let at = self.read_at()?;
        let previous = target.checked_sub(1)?;
        Some((self.committee(previous, at)?, self.committee(target, at)?))
    }
}

/// The supervised children of one beacon, aborted together.
///
/// A `Drop` impl rather than the supervisor task's own exit path, because BOTH
/// exits have to abort them and only one of them runs code: when the node aborts
/// [`Tasks::supervised`], the task's inner future is DROPPED rather than resumed,
/// so anything written after the `select` would never run. What the abort has to
/// achieve is exactly what this drop does — release the clones of the seed and key
/// stores these children hold, so the drain writers below can see their last
/// sender go and flush. commonware's own supervision does not do it for us: these
/// are the supervisor's SIBLINGS in the spawn tree, not its descendants.
struct SupervisedChildren(Vec<(&'static str, Handle<()>)>);

impl Drop for SupervisedChildren {
    fn drop(&mut self) {
        for (_, handle) in &self.0 {
            handle.abort();
        }
    }
}

/// The two handles the node owes the beacon.
///
/// Two, not eight: a beacon child dying is one fact to the node ("a subsystem
/// died, take the node down"), and which child it was belongs in the log line the
/// supervisor writes, not in the node's supervision list. And nothing else
/// crosses out: the epoch-key agreement instances the launcher starts are owned,
/// pruned and swept inside the beacon (`dkg_engine`), so the receiver that used
/// to hand them to the epoch manager is gone.
pub struct Tasks {
    /// The beacon's supervisor. Resolving means a supervised child exited, which
    /// is always fatal; aborting it aborts every child.
    pub supervised: Handle<()>,
    /// The journal writers, as ONE drain. Resolving means every writer this
    /// beacon owns has flushed and returned.
    ///
    /// MUST be awaited only once nothing can still hold an `Arc<dyn Beacon>` for
    /// this beacon: each writer is a `while let Some(_) = rx.recv().await` loop
    /// over a channel whose sender lives inside that object.
    pub drain: Handle<()>,
}

/// Spawn the beacon's supervisor over its children.
fn spawn_supervisor<E: Metrics + Spawner>(
    context: &E,
    children: Vec<(&'static str, Handle<()>)>,
) -> Handle<()> {
    // The guard is built HERE, before the task exists, and moved in — NOT inside
    // the async block. `commonware_runtime::Handle` has no `Drop` of its own
    // (`runtime/src/utils/handle.rs`: an explicit `abort()` at `:106-117` and
    // nothing else), so a supervisor aborted before its FIRST POLL would drop a
    // bare `Vec<(&str, Handle<()>)>` and leave all six children running. Built
    // outside, it is a captured field of the future from the moment the future
    // exists, so dropping the future runs `SupervisedChildren::drop` — and so does
    // the `aborted` early-return inside `spawn` itself, which never calls the
    // closure at all (`runtime/src/tokio/runtime.rs:575-578`).
    let mut children = SupervisedChildren(children);
    context
        .with_label("beacon_supervisor")
        .spawn(move |_| async move {
            if children.0.is_empty() {
                std::future::pending::<()>().await;
                return;
            }
            let labels: Vec<&'static str> = children.0.iter().map(|(l, _)| *l).collect();
            let futs: Vec<_> = children
                .0
                .iter_mut()
                .map(|(_, h)| futures::FutureExt::boxed(h))
                .collect();
            let (res, idx, _rest) = futures::future::select_all(futs).await;
            match res {
                Ok(()) => warn!(
                    child = labels[idx],
                    "beacon task exited cleanly (unexpected)"
                ),
                Err(e) => error!(child = labels[idx], error = ?e, "beacon task failed"),
            }
        })
}

/// Spawn the beacon's drain: every journal writer it owns, awaited together.
///
/// CONCURRENTLY rather than one after another, which is what the node's
/// per-drain-handle timeout used to bound separately. Nothing orders these three
/// against each other — they are independent writer loops on independent channels
/// — so racing them puts all of them inside the caller's ONE timeout instead of
/// making a stuck device pay for it three times.
///
/// SPAWNED HERE, outside any engine's spawn lineage, and that is load-bearing:
/// commonware aborts a task's DESCENDANTS, so a writer spawned under the engine
/// would be killed by the very `engine.abort()` that is supposed to release it.
/// `crates/node/src/dpos.rs` has the two-sided test.
fn spawn_drain<E: Metrics + Spawner>(
    context: &E,
    writers: Vec<(&'static str, Handle<()>)>,
) -> Handle<()> {
    context
        .with_label("beacon_drain")
        .spawn(move |_| async move {
            futures::future::join_all(writers.into_iter().map(|(label, handle)| async move {
                match handle.await {
                    Ok(()) => info!(task = label, "shutdown drain finished"),
                    Err(e) => {
                        warn!(task = label, error = ?e, "shutdown drain task did not finish cleanly")
                    }
                }
            }))
            .await;
        })
}

/// The recovery seam over BEACON_RESOLVER_CHANNEL and what it makes available.
struct ArtifactSeam {
    /// The resolver engine's start handle — aborted ONLY at process shutdown (it
    /// serves peers' fetches and drives this node's own for the whole process).
    resolver_handle: Handle<()>,
    /// The dealer-log fetch handle the ceremony and every agreement instance take.
    logs: BeaconLogs,
    /// ONE bounded acquisition of a minting epoch's artifact over this class's
    /// transport — the same [`AcquireArtifact`] the follower is built on, which is
    /// what makes the non-member's route and the follower's route one code path.
    /// It replaces the `held`/`pull` rung pair: with a single provenance tier (П-3)
    /// there is nothing for a "cheap rung" to exclude, so the local probe is just
    /// [`KeyIndex::holds_mint_of`] and this is the network half.
    acquire: AcquireMint,
    /// The `DkgActor`'s live-epoch pull — a SECOND consumer of the same
    /// [`ArtifactPull`] `acquire` uses, in the actor's fire-and-forget shape: the
    /// actor calls it from its height tick and cannot await a bounded fetch there.
    /// One `ArtifactPull` under both, so they share one per-epoch throttle.
    pull_artifact: PullArtifact,
}

/// Open the beacon recovery seam: the `commonware_resolver::p2p` engine carrying
/// both beacon subjects, and the artifact pull built over it.
///
/// The engine is built HERE rather than at the node's plane site because the pull
/// is the reason it exists: `ArtifactPull::pull` is a `fetch` on the very mailbox
/// `Engine::new` returns, and splitting the two across the crate boundary would
/// leave a mailbox on one side and the only caller that needs it on the other.
///
/// `blocker` is deliberately an isolated [`NoopBlocker`] and NOT the shared
/// oracle: a `deliver=false` on a bad dealer-log or artifact response must never
/// partition a peer from the consensus channels.
#[allow(clippy::too_many_arguments)]
fn open_artifact_seam<E, P, S, R>(
    context: &E,
    chain_id: u64,
    me: PeerPubkey,
    peers: P,
    channel: (S, R),
    store: ArtifactStore,
    committee: CommitteeSource,
    adopt_tx: mpsc::Sender<AgreedArtifact>,
    metrics: BeaconMetrics,
    log_handler: LogHandler,
) -> ArtifactSeam
where
    E: BufferPooler + Clock + CryptoRngCore + Metrics + Spawner + Clone,
    P: Provider<PublicKey = PeerPubkey>,
    S: Sender<PublicKey = PeerPubkey>,
    R: Receiver<PublicKey = PeerPubkey>,
{
    let bridge = ArtifactBridge::new(
        chain_id,
        store.clone(),
        committee,
        adopt_tx,
        metrics.clone(),
    );
    let handler = BeaconFetchHandler::new(log_handler, bridge.clone());
    let (engine, mailbox) = ResolverEngine::new(
        context.with_label("beacon_log_resolver"),
        ResolverConfig {
            peer_provider: peers,
            blocker: NoopBlocker,
            consumer: handler.clone(),
            producer: handler,
            mailbox_size: RESOLVER_MAILBOX,
            me: Some(me.clone()),
            initial: RESOLVER_INITIAL,
            timeout: RESOLVER_TIMEOUT,
            fetch_retry_timeout: RESOLVER_RETRY,
            priority_requests: false,
            priority_responses: false,
        },
    );
    let resolver_handle = engine.start(channel);

    // ONE pull for both consumers, so they share the per-epoch throttle that bounds
    // how often this node asks its peers for the same artifact.
    let pull = ArtifactPull::new(context.with_label("artifact_pull"), bridge, Some(me));
    let acquire: AcquireMint = {
        let pull = pull.clone();
        let mailbox = mailbox.clone();
        let store = store.clone();
        Arc::new(PlaneAcquire {
            pull,
            mailbox,
            store,
        })
    };
    // The `DkgActor`'s consumer of the same pull. Fire-and-forget by contract: the
    // actor calls this from its height tick, which drives every live ceremony, and
    // `pull` sleeps on the throttle and then waits out `PULL_TIMEOUT` — so it spawns
    // and returns rather than handing back a future the caller would have to await.
    //
    // `inflight` is not the rate bound; the throttle is. It stops the SPAWNS from
    // stacking: `ArtifactPull::throttle` does not de-duplicate, it sleeps until the
    // epoch's next slot and then claims it, so N concurrent callers for one epoch
    // would serialize `PULL_MIN_INTERVAL` apart instead of collapsing into one.
    let pull_artifact: PullArtifact = {
        let ctx = context.with_label("artifact_pull_live");
        let mailbox = mailbox.clone();
        let metrics = metrics.clone();
        let inflight: Arc<Mutex<BTreeSet<u64>>> = Arc::default();
        Arc::new(move |epoch: u64| {
            {
                let mut held = inflight.lock().unwrap_or_else(PoisonError::into_inner);
                if !held.insert(epoch) {
                    return;
                }
            }
            let pull = pull.clone();
            let inflight = inflight.clone();
            let metrics = metrics.clone();
            let mut resolver = mailbox.clone();
            drop(ctx.with_label("epoch").spawn(move |_| async move {
                if let Some(PullAnswer::Have(_)) = pull.pull(&mut resolver, epoch).await {
                    metrics.dkg_artifact_pull_ok.inc();
                }
                inflight
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&epoch);
            }));
        })
    };

    ArtifactSeam {
        resolver_handle,
        logs: LogFetcher::new(mailbox),
        acquire,
        pull_artifact,
    }
}

/// [`AcquireArtifact`] over `BEACON_RESOLVER_CHANNEL` — the validator half.
///
/// The verify/store/write-back body is NOT here: it lives in [`ArtifactBridge`],
/// which is the resolver's own `Consumer`, because a `deliver = false` there is what
/// costs a lying peer its standing. That is the one asymmetry against the follower's
/// [`artifact::TransportAcquire`], and it is deliberate — moving the check out of the
/// `Consumer` would move the peer-punishment decision with it.
struct PlaneAcquire<E: Clock, M> {
    pull: ArtifactPull<E>,
    mailbox: M,
    store: ArtifactStore,
}

impl<E, M> AcquireArtifact for PlaneAcquire<E, M>
where
    E: Clock + Send + Sync,
    M: commonware_resolver::Resolver<Key = BeaconFetchKey, PublicKey = PeerPubkey>
        + Clone
        + Send
        + Sync,
{
    fn fetch(&self, minted_at: u64) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            if self.store.has(minted_at) {
                return true;
            }
            let mut resolver = self.mailbox.clone();
            // `NotYet` and an exhausted walk are the same answer to this caller:
            // nobody can give it the artifact right now, so it stays unresolved.
            // The ANSWER's artifact is deliberately dropped: `ArtifactBridge::deliver`
            // verified and FILED it before it could reach here, so the store is the
            // thing to believe and re-reading it is what the caller does next.
            match self.pull.pull(&mut resolver, minted_at).await {
                Some(PullAnswer::Have(served)) => {
                    // The STORE is what to believe — `ArtifactBridge::deliver`
                    // verified and filed this before it could reach here — but the
                    // answer's own epoch is named in the line, because "a peer
                    // served something" and "a peer served THIS epoch" are the two
                    // readings a silent `true` would collapse.
                    debug!(
                        epoch = minted_at,
                        served = served.0.target_epoch,
                        "beacon: a peer served the minting epoch's artifact"
                    );
                    self.store.has(minted_at)
                }
                _ => false,
            }
        })
    }
}

/// Spawn the agreement write-back's middle hop.
///
/// It does two things and neither belongs to the actor. It publishes the agreed
/// artifact to the `DkgActor`, where its dealer-log set becomes the pinned set the
/// existing finalize rails run over — the write-back proper. It used to ALSO publish
/// the agreed key into a `BeaconKeys` store; the artifact it forwards is already in
/// [`ArtifactStore`], and that store IS the key's owner (П-3), so the publish had
/// nothing left to add.
///
/// IT NO LONGER PUBLISHES A KEY, and does not need to: the artifact it forwards is
/// already in [`ArtifactStore`] (the bridge and the instance both file before they
/// hand over), and that store IS the key's owner (П-3). What is left of this hop is
/// the write-back proper — handing the artifact to the actor so its dealer-log set
/// becomes the pinned set the finalize rails run over.
fn spawn_write_back<E>(
    context: &E,
    mut agreed_rx: mpsc::Receiver<AgreedArtifact>,
    adopt_tx: mpsc::Sender<AgreedArtifact>,
) -> Handle<()>
where
    E: Metrics + Spawner,
{
    context
        .with_label("agreement_write_back")
        .spawn(move |_| async move {
            while let Some(artifact) = agreed_rx.recv().await {
                let epoch = artifact.0.target_epoch;
                info!(
                    epoch,
                    group_public = %artifact::pk_prefix(group_public_key(&artifact.0.group_key)),
                    "beacon: epoch-key agreement artifact adopted"
                );
                if adopt_tx.send(artifact).await.is_err() {
                    warn!("beacon: the DkgActor is gone; agreement write-back stopped");
                    break;
                }
            }
            // PARK, never return. This handle is supervised, where a clean exit
            // means "a subsystem died, take the node down" — and the ordinary way
            // this loop ends is the launcher dropping its sender during shutdown,
            // which must not be the thing that cancels the node.
            std::future::pending::<()>().await;
        })
}

/// Everything the beacon plane needs that it cannot build itself.
///
/// The staking reads stay the node's — they run through its own provider and EVM
/// config, which is exactly the dependency this boundary exists to keep out of
/// the consensus crate — but they arrive as ONE [`CommitteeReads`].
pub struct ValidatorInputs<P, Se, Re, XS, XR, HS, HR>
where
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    pub chain_id: u64,
    /// This node's p2p identity, MOVED: the `DkgActor` signs its dealer logs with
    /// it and derives the plane's `me` from it.
    pub peer_keypair: Ed25519PrivateKey,
    pub bls_keypair: ValidatorBlsKeypair,
    /// `<datadir>/beacon` — where per-epoch shares and their artifacts persist.
    pub share_dir: PathBuf,
    /// The at-rest seal key for this node's DKG shares, HKDF-derived from its BLS
    /// key. `Some` ⇒ keystore mode ⇒ shares persist encrypted; `None` ⇒
    /// plaintext-dev.
    pub share_seal_key: Option<ShareSealKey>,
    /// The plane's own oracle. PRECONDITION: its `EpochTransition` tracks
    /// `committee[E−1] ∪ committee[E] ∪ committee[E+1]` as PRIMARY (4.3 — the
    /// Active registry is the secondary tier and commonware neither dials it nor
    /// caches its bodies), which is the `latest.primary` reachability both the
    /// dealer-log resolver and the body engine need: while `E` runs, the dealers
    /// of `committee[E+1]` are in it by the incoming-committee leg.
    pub peers: P,
    /// BEACON_CHANNEL halves — the DKG ceremony's own gossip.
    pub beacon_channel: (Se, Re),
    /// BEACON_RESOLVER_CHANNEL halves — the dealer-log + artifact recovery seam.
    pub resolver_channel: (XS, XR),
    /// The four plane-owned mux brokers an agreement instance sub-registers on.
    pub vote_mux: SharedMux<HS, HR>,
    pub cert_mux: SharedMux<HS, HR>,
    pub resolver_mux: SharedMux<HS, HR>,
    pub bodies_mux: SharedMux<HS, HR>,
    /// Every staking read the beacon takes, on ONE cursor.
    pub committees: Arc<dyn CommitteeReads>,
    /// The ORDERING-finalized height clock the ceremony's deal/seal geometry runs
    /// on. Ticks buffered here before the actor starts are drained by `on_height`'s
    /// monotone-max clamp.
    pub heights: mpsc::Receiver<u64>,
    /// The registered clock pair. The `DkgActor` publishes its half off the
    /// monotone clamp that merges every feeder in `heights`, so the gauge reports
    /// the clock the ceremony geometry runs on rather than whichever feeder wrote
    /// last. Arrives from the node crate, where the registry lives.
    pub plane_clock: crate::sync_metrics::PlaneClock,
    /// The node's fork-safety latch — THE one instance the executor and the epoch
    /// manager share, arriving from the node crate for the same reason
    /// `plane_clock` does (a second latch would be one nothing engages). The
    /// agreement launcher reads it at every instance spawn and waits on its
    /// 0→1 edge (`SafetyHalt::engaged_edge`, the same edge the epoch manager's
    /// engine abort waits on): a halted node starts no epoch-key agreement
    /// instance and aborts the ones it has the moment the latch engages
    /// (`dkg_engine`). The `DkgActor` does not hold it.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// The plane's frozen `(dpos_activation, epoch_interval)`, as a WATCH.
    ///
    /// `None` is the [`GeometryUnfrozen`](super::WithheldReason::GeometryUnfrozen)
    /// state, not an error:
    /// `build` neither awaits it nor fails on it, the actor waits for the first
    /// `Some` and starts then, and `can_participate` says why it is withholding
    /// meanwhile. It used to be a `BoxFuture` over a one-shot `Notify` — so a
    /// wake-up that raced the freeze read `None`, logged, and left the node with
    /// no `DkgActor` for the life of the process.
    pub geometry: watch::Receiver<Option<(u64, u64)>>,
    /// Prefix of every storage partition the plane opens: the epoch-key
    /// agreement journals (`{prefix}dkg_epoch_{E}`, see
    /// [`crate::dpos::AGREEMENT_JOURNAL_PARTITION_PREFIX`]) and the key / seed / artifact
    /// journals (`{prefix}` ‖ [`MINT_MEMO_PARTITION`] etc., see
    /// [`journal_partition`]). Production passes `""`; the in-crate testbed a
    /// per-node prefix, so N planes on one in-memory `Storage` do not write one
    /// journal.
    pub partition_prefix: String,
}

/// A plane journal's partition under the plane's prefix: the production names
/// verbatim for the empty prefix, `{prefix}{base}` otherwise.
pub(crate) fn journal_partition(prefix: &str, base: &str) -> String {
    format!("{prefix}{base}")
}

/// Build the always-on beacon.
///
/// One call, one `Arc<dyn Beacon>` and one [`Tasks`]: the ceremony store, the
/// dealer-log index, the artifact store, the confirmation pool, the resolver
/// seam and the agreement launcher are all created here and none of them crosses
/// back out. The node keeps the network, the mux brokers, the finalized-height
/// poller and the staking reads — everything whose dependency runs the other way.
pub async fn build<E, P, Se, Re, XS, XR, HS, HR>(
    context: &E,
    cfg: ValidatorInputs<P, Se, Re, XS, XR, HS, HR>,
) -> eyre::Result<(Arc<dyn Beacon>, Tasks)>
where
    E: BufferPooler + Clock + CryptoRngCore + Metrics + Spawner + Storage + Clone + Send + 'static,
    P: Provider<PublicKey = PeerPubkey> + Clone + Sync + 'static,
    Se: Sender<PublicKey = PeerPubkey>,
    Re: Receiver<PublicKey = PeerPubkey>,
    XS: Sender<PublicKey = PeerPubkey>,
    XR: Receiver<PublicKey = PeerPubkey>,
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    let ValidatorInputs {
        chain_id,
        peer_keypair,
        bls_keypair,
        share_dir,
        share_seal_key,
        peers,
        beacon_channel,
        resolver_channel,
        vote_mux,
        cert_mux,
        resolver_mux,
        bodies_mux,
        committees,
        heights,
        plane_clock,
        safety_halt,
        geometry,
        partition_prefix,
    } = cfg;
    // The four closures the internals still speak, all projected off ONE
    // `CommitteeReads`, so every one of them takes its state hash from the SAME
    // `read_at` (Д-7). What that buys is precise, and less than "atomic": the qual
    // bit can no longer be read at a DIFFERENT class of height than the committee
    // — which is what it used to be, finalized-only against `max(fin, live)`.
    // Two of these closures called a beat apart can still straddle a block, and
    // the one place where that would be a defect is the ceremony-start decision,
    // which is why `committee_pair` resolves the cursor once for both epochs
    // instead of being two `committee` calls at the call site.
    let committee_for: CommitteeFor = {
        let reads = committees.clone();
        Arc::new(move |epoch: u64| reads.committee(epoch, reads.read_at()?))
    };
    let committee_source: CommitteeSource = {
        let reads = committees.clone();
        Arc::new(move |epoch: u64| reads.committee_bls(epoch, reads.read_at()?))
    };
    // The frozen `changed` bit, one closure for both node classes — see
    // [`changed_bit`] for why the `committed` leg is dropped rather than guarded
    // (Д-7).
    let changed: ChangedAt = changed_bit(committees.clone());
    let me = peer_keypair.public_key();
    let share_state = match share_seal_key {
        Some(key) => ShareState::Encrypted(key),
        None => ShareState::Plaintext,
    };

    // Shared live-DKG store, reloaded from the share dir ONCE.
    let ceremony_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
    let share_notify = Arc::new(Notify::new());
    let reloaded = share_state::load_all(&share_dir, &share_state);
    if !reloaded.is_empty() {
        if let Ok(mut store) = ceremony_store.write() {
            for (epoch, share) in reloaded {
                info!(epoch, "beacon: reloaded persisted live-DKG share from disk");
                store.insert(epoch, share);
            }
        }
    }

    // The shared dealer-log hash index: the `DkgActor` publishes idx→hash of every
    // log it has recorded, and the agreement plane proposes over it (its
    // share-confirmations state the same set).
    let recorded_dkg_logs: DkgLogIndex = Arc::new(RwLock::new(BTreeMap::new()));

    // Beacon counters — registered ONCE here (the persistent layer); cloned (never
    // re-registered) into the `DkgActor` + each per-epoch signer engine.
    let metrics = BeaconMetrics::default();
    metrics.register(context);

    let dkg_namespace = seed_namespace(&fluent_namespace(chain_id));
    // The epoch-key agreement plane's shared state, all of it created ONCE and
    // handed to BOTH the `DkgActor` and every instance the launcher starts.
    //
    // `confirm_pool` carries the namespace share-confirmations are signed under, so
    // handing the same pool to both sides is what makes it impossible for them to
    // disagree about it — a second pool built from a different base would reject
    // every honest confirmation and the entry bar would never be met, silently.
    let confirm_pool = ConfirmPool::new(&dkg_namespace);
    let (pinned_tx, pinned_rx) = mpsc::channel::<PinnedRequest>(PINNED_MAILBOX);
    // The actor's dealing-closed edge. Bounded and `try_send`-driven: the actor
    // re-announces every open target on each height tick, so a full channel costs a
    // tick of latency and never a lost instance.
    let (agreement_request_tx, agreement_request_rx) = mpsc::channel::<u64>(EDGE_MAILBOX);
    // The actor's epoch clock as the launcher sees it (`Wiring::epoch_clock`):
    // the cutoff epoch, published on change. Epoch `0` until the first tick,
    // whose band is empty.
    let (epoch_clock_tx, epoch_clock_rx) = watch::channel(0u64);
    // THE DURABLE MINT MEMO, and it opens where the key journal used to. It is the
    // PRECONDITION of that journal's deletion, not a replacement for it: the journal
    // held `epoch → pk` and could not answer a carry epoch at all (W1 did that, and
    // W1 is gone); this holds `epoch → minting epoch`, which is what makes the
    // durable artifact store ADDRESSABLE without a chain read. See
    // `artifact::open_mint_memo` for what it closes and the one case it does not.
    let (mints, mint_writer) = artifact::open_mint_memo(
        context.with_label("mint_memo"),
        context.with_label("mint_memo_writer"),
        &journal_partition(&partition_prefix, MINT_MEMO_PARTITION),
        changed.clone(),
    )
    .await?;

    // The `round → σ` index (`crate::beacon::seed_index`), the ONE owner of the
    // seed fact. Every door writes it through `Beacon::observe_certificate`; the
    // executor's derive and the epoch manager's boundary base read it
    // synchronously. Cross-epoch singleton — created BEFORE the executor (its
    // first consumer below).
    //
    // The durable store is opened and REPLAYED here, ahead of the executor,
    // `FluentApp` and every engine, so no consumer can observe a
    // half-rehydrated store. It is a singleton with the store it backs: a
    // second handle over the same partition would be a dual-writer, one of
    // which prunes a blob the other still holds open.
    //
    // THE WRITER MUST OUTLIVE `engine.abort()`, or the drain that is supposed to
    // flush its tail kills it instead. commonware supervision aborts a task's
    // descendants, and descendancy is SPAWN LINEAGE, not label path: what dies
    // with the engine task is what that task spawned from its OWN context.
    // (Verified on the runtime, not assumed — `crates/node/src/dpos.rs` has the
    // two-sided test, written that way because a one-sided version passed while
    // the writer sat under a context labelled `outer_engine`.)
    //
    // Spawning here satisfies that trivially: `beacon::build` runs before any
    // engine task exists and has no access to one, so no arrangement at the call
    // site is required to keep the property. That is the improvement over the
    // previous shape, which threaded a second `seed_writer_context` into
    // `OuterBuilder::build` for the caller to get right.
    //
    // The label path DOES change: the families become `seed_journal_*` instead
    // of `outer_engine_seed_journal_*`. Checked before moving — the devnet
    // harness asserts the durable store from a LOG line, not from any journal
    // metric, so nothing scrapes the old names.
    let (seed_store, seed_writer) = super::seed_journal::open(
        context.with_label("seed_journal"),
        context.with_label("seed_journal_writer"),
        &journal_partition(&partition_prefix, SEED_JOURNAL_PARTITION),
        super::seed_index::SEED_RETENTION,
    )
    .await?;

    let (agreed_tx, agreed_rx) = mpsc::channel::<AgreedArtifact>(EDGE_MAILBOX);
    let (adopt_tx, artifacts_rx) = mpsc::channel::<AgreedArtifact>(EDGE_MAILBOX);
    // The instance's other verdict — a certified body it could not resolve —
    // goes straight to the actor, which owns the pull that heals it (R-026).
    let (body_lost_tx, body_lost_rx) = mpsc::channel::<u64>(EDGE_MAILBOX);

    // The `LogHandler` bridges the resolver engine's Producer/Consumer to the
    // `DkgActor` run loop, which owns the ceremony state single-threaded.
    let (log_resolver_tx, log_resolver_rx) = mpsc::channel::<LogMessage>(RESOLVER_MAILBOX);
    let log_handler = LogHandler::new(log_resolver_tx);

    // The per-epoch artifact store: RAM for every in-process reader, plus a durable
    // mirror so the value survives the restart that today loses it outright. ONE
    // instance per process — a second handle over this partition is a dual-writer.
    let (artifact_store, artifact_writer_handle) = artifact::open(
        context.with_label("artifact_store"),
        context.with_label("artifact_store_writer"),
        &journal_partition(&partition_prefix, ARTIFACT_JOURNAL_PARTITION),
    )
    .await?;
    // The store OWNS the `Conflict` witness, on disk as well as in RAM: a second
    // certified value it notes is written as `beacon-conflict-e<E>.bin` in the
    // same directory the actor's `recover` reads the verdict back from, the
    // instant it is noted — so a restart between the note and the actor's next
    // tick cannot forget it (Д-А1-20).
    let artifact_store = artifact_store.with_conflict_dir(share_dir.clone());
    // THERE IS NO SECOND REFILL ROUTE ANY MORE, and its absence is П-3. The share
    // file used to carry a copy of the agreed artifact, and this is where that copy
    // was read back into the store — so an epoch whose artifact journal record never
    // synced still came back with a locally-sourced `PK_E`. The copy is gone
    // (`share_state`), so the artifact journal's own rehydration above is the whole
    // of what a restart recovers, and an epoch it lost is acquired from peers
    // (`DkgActor::drive_acquisition`). The liveness trade is named in
    // `share_state`'s module doc. Nor is there a replay of the store INTO the
    // actor: the store is the owner of the epoch's artifact and the actor READS
    // it — on the tick an epoch is decided (`DkgActor::recover`) and on every
    // tick after (`reconcile_with_store`) — so a member that went down between
    // adopting an artifact and finalizing over it is served by that read.

    let ArtifactSeam {
        resolver_handle,
        logs,
        acquire,
        pull_artifact,
    } = open_artifact_seam(
        context,
        chain_id,
        me.clone(),
        peers.clone(),
        resolver_channel,
        artifact_store.clone(),
        committee_source.clone(),
        agreed_tx.clone(),
        metrics.clone(),
        log_handler,
    );
    // The ONE owner of `PK_epoch` and the polynomial, assembled where its two halves
    // first exist together: the durable artifact store and the durable mint memo.
    let key_index = KeyIndex::new(artifact_store.clone(), mints);

    // The actor's READ of the artifact store (`recover(E)` and the per-tick
    // `reconcile_with_store`): the held payload and the divergent second value, if
    // the store ever noted one. It used to read the boundary block at
    // `epoch_start(E)`, which was a chicken-and-egg — the heal exists for a member
    // that could not enter `E`, and `E`'s own first block is what such an epoch
    // does not produce.
    let outcome_at: AgreedOutcomeAt = {
        let store = artifact_store.clone();
        Arc::new(move |epoch: u64| {
            store.view(epoch).map(|(held, divergent)| StoredArtifact {
                held: held.0.clone(),
                divergent,
            })
        })
    };

    // One clone for the serving read `consensus_getEpochArtifact` answers from,
    // and one geometry receiver for `can_participate`'s unfrozen answer. Taken
    // here because the store moves into the agreement launcher further down and
    // the actor's spawn wrapper takes the watch.
    let artifact_store_for_serving = artifact_store.clone();
    // ONE edge, taken BEFORE the store moves into the agreement launcher. It used
    // to be two independent subscriptions — one for the wake-up bridge, one for a
    // seed-promoter task — and the pair was a RACE rather than a redundancy: both
    // fired on the same artifact insert, in either order, so a consumer woken by
    // `KeyAvailable` could re-read the seed index before the promote had run. The
    // settle is the bridge's first act now (`LiveBeacon::settle_pending`), so the
    // wake-up cannot outrun the σ it unlocks.
    let bridge_key_edge = artifact_store.subscribe();
    let geometry_for_probe = geometry.clone();

    // The persistent `DkgActor` — spawned ONCE, runs for the whole process. It is
    // constructed AFTER the plane has frozen the geometry, so it takes plain
    // `(activation, interval)` from the single in-plane source and never re-reads
    // the chain. Height ticks accumulate in `heights` meanwhile (bounded buffer)
    // and are drained by `on_height`'s monotone-max clamp once the actor runs — the
    // first epoch boundary is one interval away (≫ the freeze latency), so no
    // deal/seal is missed.
    let dkg_handle = {
        let (sender, receiver) = beacon_channel;
        let committee_for = committee_for.clone();
        let ceremony_store = ceremony_store.clone();
        let share_notify = share_notify.clone();
        let actor_metrics = metrics.clone();
        let recorded = recorded_dkg_logs.clone();
        let confirms = confirm_pool.clone();
        let logs = logs.clone();
        context.with_label("dkg_actor").spawn(move |c| async move {
            // UNFROZEN is a state, not an error: wait for the first `Some` and
            // start then. The predecessor awaited a one-shot `Notify` and read the
            // geometry once — so a wake-up that raced the freeze read `None`,
            // logged, and returned, leaving the node with no `DkgActor` for the
            // life of the process. Height ticks accumulate in `heights` meanwhile
            // (bounded buffer) and are drained by `on_height`'s monotone-max clamp.
            let mut geometry = geometry;
            let (activation, interval) = loop {
                if let Some(frozen) = *geometry.borrow_and_update() {
                    break frozen;
                }
                if geometry.changed().await.is_err() {
                    // Every sender is gone: the node is shutting down.
                    error!("beacon plane: the geometry watch closed while unfrozen");
                    return;
                }
            };
            // Every edge of the actor, in one place and none of them optional.
            let actor = DkgActor::new(
                dkg_namespace,
                peer_keypair,
                sender,
                receiver,
                committee_for,
                ceremony_store,
                share_notify,
                activation,
                interval,
                actor_metrics,
                share_state,
                Wiring {
                    resolver: logs,
                    resolver_rx: log_resolver_rx,
                    changed: changed.clone(),
                    share_dir,
                    plane_clock,
                    outcome_at,
                    pull_artifact,
                    recorded_dkg_logs: recorded,
                    confirms,
                    pinned_rx,
                    agreement_tx: agreement_request_tx,
                    epoch_clock: epoch_clock_tx,
                    artifacts_rx,
                    body_lost_rx,
                    #[cfg(test)]
                    fixture: None,
                },
            );
            actor.run(heights, c).await
        })
    };

    // The epoch-key agreement launcher. It owns everything the `DkgActor` cannot
    // reach — the four mux sub-channel registrations, the staking committee read
    // and the runtime context an instance is spawned on — turns the actor's
    // dealing-closed edge into a running instance, and OWNS that instance from
    // there: pruned on the actor's epoch clock, its journal partition swept by
    // the launcher's own band sweep, aborted on the SafetyHalt latch's own edge
    // and refused outright once it is engaged. Nothing about an instance leaves
    // the beacon.
    let agreement_launcher_handle = spawn_agreement_launcher(
        context.clone(),
        AgreementPlaneConfig {
            chain_id,
            keypair: bls_keypair,
            me,
            peers,
            logs,
            recorded: recorded_dkg_logs,
            pinned_requests: pinned_tx,
            confirms: confirm_pool,
            metrics: metrics.clone(),
            artifacts: artifact_store,
            committee: committee_source,
            mailbox_size: AGREEMENT_MAILBOX,
            timeouts: AgreementTimeouts::coarse(),
            partition_prefix,
            body_lost: Some(body_lost_tx),
            #[cfg(test)]
            probe: None,
        },
        AgreementMuxes {
            vote: vote_mux,
            cert: cert_mux,
            resolver: resolver_mux,
            bodies: bodies_mux,
        },
        agreement_request_rx,
        agreed_tx,
        epoch_clock_rx,
        safety_halt,
    );

    // The agreement write-back, armed IN PLACE. It used to be handed out
    // unjoined (`BeaconWriteBack`) for the node to arm after the consensus layer
    // had created the key store; the store is created above now, so the two ends
    // meet here and the arm-later dance is gone.
    let write_back_handle = spawn_write_back(context, agreed_rx, adopt_tx);

    // Everything randomness-shaped, behind ONE handle. This is the only place
    // where all of its inputs exist at once — the ceremony store, the frozen
    // `dkgQual` arbiter, the key store, the seed store and the two agreement
    // rungs — and none of them crosses back out.
    let seed_events = seed_store.events().clone();
    let randomness = LiveBeacon::build(LiveBeaconConfig {
        artifacts: artifact_store_for_serving,
        geometry: geometry_for_probe,
        seeds: seed_store,
        keys: key_index.clone(),
        ceremony: ceremony_store.clone(),
        acquire: Some(acquire),
        metrics: metrics.clone(),
        chain_id,
    });

    // The wake-up bridge. `record_seed` fires its own class from inside the
    // beacon; these two are written from OTHER tasks through a bare `Notify` —
    // an accepted `ArtifactStore::insert` and the `DkgActor`'s share edge — and `notify_one`
    // wakes exactly ONE waiter, so this task being their SOLE waiter is what lets
    // any number of consumers subscribe without swallowing each other's edges.
    //
    // SUPERVISED: with it dead the executor never learns a key landed and the
    // epoch manager never learns its participation changed, which is a silent
    // stall rather than a visible failure.
    let bridge_handle = {
        let key_edge = bridge_key_edge;
        // The settle rides this task's KEY arm — see `LiveBeacon::settle_pending`
        // for why it is not a task of its own any more.
        let settle = randomness.clone();
        let participation_edge = share_notify.clone();
        // The SAME publisher `SeedIndex::record` fires the seed class into, so the
        // three classes reach every consumer over one subscription.
        let events = seed_events.clone();
        context
            .with_label("beacon_event_bridge")
            .spawn(move |_| async move {
                loop {
                    let key = key_edge.notified();
                    let participation = participation_edge.notified();
                    tokio::pin!(key, participation);
                    let event = tokio::select! {
                        () = &mut key => {
                            // BEFORE the publish, never after: a consumer re-reads
                            // state on the wake-up, and a σ the landed key has just
                            // made servable must already be `Verified` when it does.
                            settle.settle_pending();
                            BeaconEvent::KeyAvailable
                        }
                        () = &mut participation => BeaconEvent::ParticipationChanged,
                    };
                    // `Err` means no subscriber right now, which is ordinary at
                    // startup and at shutdown; the next consumer re-reads state on
                    // its first pass either way.
                    let _ = events.send(event);
                }
            })
    };

    let mut writers: Vec<(&'static str, Handle<()>)> = vec![("seed_journal_writer", seed_writer)];
    if let Some(writer) = artifact_writer_handle {
        writers.push(("artifact_store_writer", writer));
    }
    if let Some(writer) = mint_writer {
        writers.push(("mint_memo_writer", writer));
    }
    let tasks = Tasks {
        supervised: spawn_supervisor(
            context,
            vec![
                ("dkg", dkg_handle),
                ("beacon_resolver", resolver_handle),
                ("agreement_launcher", agreement_launcher_handle),
                ("agreement_write_back", write_back_handle),
                ("event_bridge", bridge_handle),
            ],
        ),
        drain: spawn_drain(context, writers),
    };
    Ok((randomness, tasks))
}

// ---------------------------------------------------------------------------
// The `--cert-follow` follower's beacon
// ---------------------------------------------------------------------------
//
// ONE `Randomness` IMPLEMENTATION FOR BOTH NODE CLASSES, which is what PLAN row
// 5.2 asks for when it says to delete the follower's seed half and the file it
// lived in. What a follower is, is now a CONFIGURATION of `LiveBeacon` rather
// than a second implementation of the surface:
//
//   * an EMPTY `CeremonyStore` (`surface::keyless_ceremony`). Every
//     material-bound answer of `BeaconOracle` goes through `with_material`, which
//     returns `None` when this node holds no share of the mint — the exact
//     permanent negatives the deleted `KeyOnlyOracle` returned by type. `KeyIndex`
//     still resolves the epoch's mint and its public polynomial out of the
//     artifact, so `verify_seed` — the one answer a follower CAN give — is
//     unchanged: both oracles' bodies were already byte-identical.
//   * RAM-only stores: no `share_dir` on this path and no durable partition to
//     open. What a restart loses is one fetch per epoch over a link the follower
//     holds open anyway, including the mint memo.
//   * `acquire` over this class's own transport, and the KEY WANT wired into
//     `hold_seed` (`LiveBeacon::wire_want`): a follower has no epoch manager, so
//     the `Pending` verdict is the ONLY thing that can ask for a key.
//   * no DKG, no agreement plane, no muxes — the `Withheld`
//     verdicts follow from the empty share store rather than from a second type.
//     The geometry watch is the one item of that list that STAYS, and is SET on
//     purpose rather than left at its default: `build_resolved` hands the beacon
//     a resolved `watch::channel(Some((0, 1))).1` (`plane.rs:1206-1210`) exactly
//     because a follower freezes no `(activation, interval)` of its own and never
//     asks to participate, so `share_probe`'s `GeometryUnfrozen` refinement must
//     not fire — `NoUsableShare` is the true story for a node class that runs no
//     ceremony. The consequence is confined to the `WithheldReason` LABEL, not
//     to behaviour; dropping the field would only mislabel it.
//
// # What this closed, and still closes
//
// A follower used to run `surface::absent`, whose `ensure_key` answers `None` at
// both efforts for the life of the process. Every certificate it ingested
// therefore took VOTE-ONLY admission: the attributable `2f+1` multisig quorum was
// checked, the seed slot was not, so a tampered or cleared seed riding a valid
// quorum was admitted in silence. Nothing the verification needs was missing —
// the chain id, an rng and the `committee[epoch]` read are all things a follower
// already has. What was missing was a DELIVERY ROUTE for the artifact, which the
// caller supplies as [`ArtifactFetch`] over the one peer relationship a follower
// has: its cert upstream.
//
// # Trust
//
// The upstream is trusted for DELIVERY and for nothing else. A fetched artifact
// is checked against `committee[epoch]` read from THIS node's own chain state, by
// the same `verify_artifact_for_epoch` a validator's pull seam uses — so a lying
// upstream is caught here exactly as a lying peer is caught there.

/// Depth of the want channel between `Randomness::hold_seed` and the fetch task.
/// Wants are re-issued on every certificate (~1/s) for as long as the epoch stays
/// unresolved, so a full channel costs nothing: the drop is re-asked a second
/// later, and dropping is what keeps the verdict off the network.
const WANT_MAILBOX: usize = 16;

/// One artifact fetch over the follower's cert upstream, by MINTING epoch — the
/// BYTES half of an acquisition, supplied by the node.
///
/// It is [`artifact::ArtifactBytes`] under this seam's own name, and it stays a closure
/// rather than becoming the trait journal 5.0's Д-11 asked for, for a reason that
/// is a boundary and not a preference: the only supplier is
/// `crate::dpos::CertUpstream::get_epoch_artifact`, whose call site is production
/// code in a file this row may not write. The trait Д-11 wanted DOES exist —
/// [`AcquireArtifact`], with the two implementors that justify it — and this alias
/// is now its argument rather than a second abstraction.
pub type ArtifactFetch = artifact::ArtifactBytes;

/// What [`build_follower`] needs that it cannot build itself. Every field is a
/// capability the node already holds; none of them is beacon state.
///
/// `--cert-follow` has no keys, no muxes and no DKG, so this is not a narrowing of
/// [`ValidatorInputs`] but a different set: what it does have is the artifact
/// upstream, and the artifact half is all a follower's beacon does.
///
/// There is no `partition_prefix`: the follower is RAM-only by decision, and what
/// a restart costs it is one fetch per epoch over a link it holds open anyway.
pub struct FollowerInputs {
    pub chain_id: u64,
    /// Every staking read the follower's beacon takes, on ONE cursor — the same
    /// trait the validator plane is handed.
    pub committees: Arc<dyn CommitteeReads>,
    /// The delivery route. See [`ArtifactFetch`].
    pub fetch: ArtifactFetch,
}

/// [`FollowerInputs`] with its staking reads already projected into the two
/// closures the internals speak. Private: it exists so the body below is unchanged
/// by the boundary move, and so this file's tests can build the beacon over canned
/// closures without going through [`CommitteeReads`].
struct ResolvedFollowerInputs {
    chain_id: u64,
    committees: CommitteeSource,
    changed: ChangedAt,
    fetch: ArtifactFetch,
}

/// What [`build_resolved`] hands back. Private: the two halves leave through
/// [`build_follower`]'s [`Tasks`].
struct FollowerBeacon {
    randomness: Arc<dyn Beacon>,
    /// The SAME object as `randomness`, kept concrete for this file's own tests:
    /// the refusal latch is a `Randomness` method and the counters are private to
    /// the provider, neither of which `dyn Beacon` can reach.
    #[cfg(test)]
    provider: Arc<LiveBeacon>,
    fetch_handle: Handle<()>,
}

/// Build the follower's beacon.
///
/// Registers the beacon metric families on `context` — a follower is the sole
/// owner of them on its node class.
pub fn build_follower<E>(context: &E, cfg: FollowerInputs) -> (Arc<dyn Beacon>, Tasks)
where
    E: Clock + Metrics + Spawner + Clone + Send + 'static,
{
    let committee_source: CommitteeSource = {
        let reads = cfg.committees.clone();
        Arc::new(move |epoch: u64| reads.committee_bls(epoch, reads.read_at()?))
    };
    let changed = changed_bit(cfg.committees.clone());
    let beacon = build_resolved(
        context,
        ResolvedFollowerInputs {
            chain_id: cfg.chain_id,
            committees: committee_source,
            changed,
            fetch: cfg.fetch,
        },
    );
    (
        beacon.randomness,
        Tasks {
            supervised: beacon.fetch_handle,
            // A follower opens no journal partition — RAM-only artifact store,
            // RAM-only seed index — so there is nothing to flush at shutdown.
            drain: context
                .with_label("beacon_drain")
                .spawn(move |_| async move {}),
        },
    )
}

/// The body, over the closures the internals speak.
fn build_resolved<E>(context: &E, cfg: ResolvedFollowerInputs) -> FollowerBeacon
where
    E: Clock + Metrics + Spawner + Clone + Send + 'static,
{
    let metrics = BeaconMetrics::default();
    metrics.register(context);

    let store = ArtifactStore::new();
    let keys = KeyIndex::new(store.clone(), artifact::MintIndex::new(cfg.changed));

    // THE SAME ACQUISITION THE NON-MEMBER VALIDATOR USES, over this class's
    // transport. Throttle, decode, verify against `committee[minted_at]` and file —
    // one body in `artifact`, so the check that makes a lying upstream and a lying
    // peer the same non-event cannot be two different checks. No write-back hop:
    // this node class has no actor to adopt a pinned set into.
    let acquire: AcquireMint = Arc::new(artifact::TransportAcquire::new(
        cfg.chain_id,
        cfg.committees,
        cfg.fetch,
        store.clone(),
        None,
        context.clone(),
        metrics.clone(),
    ));

    let (want_tx, want_rx) = mpsc::channel(WANT_MAILBOX);
    // Captured before the task exists, as the store's own edge requires: a fill
    // landing between the spawn and the task's first poll would be lost to a handle
    // taken inside the loop, and an artifact arrives ONCE per epoch — unlike a want,
    // nothing re-issues it a second later.
    let key_edge = store.subscribe();
    let randomness = LiveBeacon::build(LiveBeaconConfig {
        // RAM-only, like the artifact store above and for the same reason: this
        // path opens no journal partition. What a restart loses is σ the next
        // certificate carries anyway.
        seeds: super::seed_index::SeedIndex::new(),
        keys,
        // THE ONE FIELD THAT MAKES THIS A FOLLOWER — see the section head.
        ceremony: super::surface::keyless_ceremony(),
        acquire: Some(acquire),
        metrics,
        chain_id: cfg.chain_id,
        artifacts: store,
        // A follower freezes no `(activation, interval)` of its own and never asks
        // to participate, so the only reader of this watch — `share_probe`'s
        // `GeometryUnfrozen` refinement — must not fire: `NoUsableShare` is the
        // true story for a node class that runs no ceremony.
        geometry: watch::channel(Some((0, 1))).1,
    });
    // Before the `Arc` is handed to anything, which is what makes the write-once
    // cell sound.
    randomness.wire_want(want_tx);
    let fetch_handle = {
        // WEAK on purpose: the task holds the receiving end of `want_tx`, so an
        // `Arc` here would keep the sender alive through its own owner and the
        // loop could never tell shutdown from idleness.
        let provider = Arc::downgrade(&randomness);
        context
            .with_label("follower_artifact_fetch")
            .spawn(move |_| run_fetcher(want_rx, key_edge, provider))
    };

    FollowerBeacon {
        #[cfg(test)]
        provider: randomness.clone(),
        randomness,
        fetch_handle,
    }
}

/// The FROZEN `changed` bit over a [`CommitteeReads`], for both node classes.
///
/// The `committed` leg the raw read carries is dropped here rather than guarded —
/// see [`ChangedAt`] for why that is Д-7 resolved: the module answers a record only
/// for an epoch whose committee it read, so `committed` is unconditionally `true`
/// and `!(bit || committed) ⇒ None` had one live meaning left.
pub(super) fn changed_bit(reads: Arc<dyn CommitteeReads>) -> ChangedAt {
    Arc::new(move |epoch: u64| {
        let at = reads.read_at()?;
        reads.dkg_qual(epoch, at).map(|(bit, _committed)| bit)
    })
}

/// The off-path half, and the ONLY place a follower touches the network for a key.
///
/// Sequential by construction: one acquisition at a time, so a slow upstream costs
/// latency and never a fan-out. The per-epoch throttle lives inside
/// `TransportAcquire`, shared with every other consumer of it. The acquisition
/// itself is `ensure_key(Thorough)` — the SAME operation the epoch manager calls on
/// a validator, which is what keeps "how a key is obtained" one body.
///
/// It carries the SETTLE of held σ on a second arm, and that arm is this class's
/// copy of the validator plane's event bridge: what decides whether a held σ can be
/// served is an artifact landing, and the STORE's own edge is the trigger rather
/// than this task's fetch result — an epoch also resolves off an artifact adopted
/// for a DIFFERENT epoch (a carry), and a σ waiting on that one would otherwise
/// never be re-checked. The settle rides this task instead of one of its own
/// because the two share a single failure story — a follower that has stopped using
/// `PK_epoch` — and a second task would have to be supervised separately to tell
/// the same thing.
async fn run_fetcher(
    mut want_rx: mpsc::Receiver<u64>,
    key_edge: Arc<Notify>,
    provider: std::sync::Weak<LiveBeacon>,
) {
    loop {
        let epoch = tokio::select! {
            want = want_rx.recv() => match want {
                Some(epoch) => epoch,
                None => break,
            },
            _ = key_edge.notified() => {
                // Gone means the last beacon handle dropped, i.e. shutdown.
                let Some(randomness) = provider.upgrade() else { break };
                // BEFORE the publish, never after — the plane's bridge takes the
                // same order and for the same reason: a consumer re-reads state on
                // the wake-up, and a σ the landed key has just made servable must
                // already be `Verified` when it does.
                randomness.settle_pending();
                // This task is the SOLE waiter on the store's own notifier, so it is
                // also the only place that can turn an artifact landing into the
                // beacon's `KeyAvailable` wake-up.
                let _ = super::surface::Randomness::events(randomness.as_ref())
                    .send(BeaconEvent::KeyAvailable);
                continue;
            }
        };
        let Some(randomness) = provider.upgrade() else {
            break;
        };
        let _ = super::surface::Randomness::ensure_key(
            randomness.as_ref(),
            epoch,
            super::surface::PinEffort::Thorough,
        )
        .await;
    }
    // PARK, never return: this handle is supervised, where a clean exit means "a
    // subsystem died, take the node down". The loop ends only when the last beacon
    // handle drops, which is shutdown, and that must not be the thing that cancels
    // the node.
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production journal names are the bare constants: an empty prefix
    /// changes nothing, and the testbed's per-node prefix lands in front of
    /// every one of them.
    #[test]
    fn journal_partitions_are_the_production_names_under_the_empty_prefix() {
        assert_eq!(
            journal_partition("", MINT_MEMO_PARTITION),
            "beacon-mint-metadata"
        );
        assert_eq!(
            journal_partition("", SEED_JOURNAL_PARTITION),
            "beacon-seed-ordinal"
        );
        assert_eq!(
            journal_partition("", ARTIFACT_JOURNAL_PARTITION),
            "beacon-artifact-metadata"
        );
        assert_eq!(
            journal_partition("node3-", SEED_JOURNAL_PARTITION),
            "node3-beacon-seed-ordinal"
        );
    }
}

/// The follower beacon's own tests, moved with the builder above when row 5.2
/// deleted `beacon/follower.rs`. Kept a module of their own rather than merged
/// into [`tests`]: the two have disjoint fixtures and the import blocks do not
/// overlap.
#[cfg(test)]
mod follower_tests {
    use super::*;
    use crate::beacon::{
        surface::{PinEffort, Randomness},
        DataFault,
    };
    use crate::{
        beacon::{
            artifact::decode_artifact,
            dkg_agree::{AgreedArtifact, DkgProposal},
            outcome::DkgOutcome,
            surface::DealtOracle,
            BeaconEvent, Observed, ObservedCertificate,
        },
        digest::Digest,
    };
    use alloy_primitives::{Address, B256};
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{group::Share, sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
    };
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{
        ordered::{BiMap, Set},
        N3f1, TryCollect as _,
    };
    use fluentbase_bls::oracle::SeedOracle;
    use fluentbase_bls::{
        beacon::dkg_namespace, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
        scheme::build_verifier, BlsPubkey, EpochCommittee, PeerPubkey, Scheme as BlsScheme,
    };
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const CHAIN_ID: u64 = 20_994;
    /// The epoch whose committee MINTED the key, so `dkgQual[TARGET]` is set and
    /// the ladder asks for exactly this epoch's artifact.
    const TARGET: u64 = 9;
    const N: usize = 4;

    struct Committee {
        peers: Vec<Ed25519PrivateKey>,
        bls: Vec<ValidatorBlsKeypair>,
    }

    fn committee(seed: u64) -> Committee {
        let mut rng = StdRng::seed_from_u64(seed);
        Committee {
            peers: (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect(),
            bls: (0..N)
                .map(|_| ValidatorBlsKeypair::generate(&mut rng))
                .collect(),
        }
    }

    impl Committee {
        fn bimap(&self) -> BiMap<PeerPubkey, BlsPubkey> {
            self.peers
                .iter()
                .zip(self.bls.iter())
                .map(|(p, b)| {
                    (
                        p.public_key(),
                        BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                    )
                })
                .try_collect()
                .expect("unique committee")
        }

        /// Exactly what the follower's own `committee_source` closure hands back.
        fn epoch_committee(&self, epoch: u64) -> EpochCommittee {
            let snap = fluentbase_staking_reader::reader::ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0x11),
                block_number: 4_096,
                epoch,
                validators: self
                    .peers
                    .iter()
                    .zip(self.bls.iter())
                    .enumerate()
                    .map(|(i, (p, b))| ValidatorWithKeys {
                        address: Address::repeat_byte(i as u8 + 1),
                        keys: ConsensusKeys {
                            bls_pubkey: BlsPubkey::decode(b.public_bytes().as_slice())
                                .expect("bls pubkey"),
                            peer_pubkey: p.public_key(),
                            activation_epoch: 0,
                        },
                        tombstoned: false,
                    })
                    .collect(),
                weights: None,
            };
            crate::scheme::epoch_committee_from_snapshot(&snap).expect("committee")
        }

        /// A real DKG over THIS committee's peers, so the artifact the follower
        /// adopts and the σ its certificates carry stand under one key. `deal`
        /// indexes shares by the player's position in the commonware-sorted
        /// `Set`, which is the order `build_signer` asserts a member's share
        /// index against — hence the per-peer lookup rather than a bare `values`.
        fn deal(&self) -> (DkgOutcome, Vec<Share>) {
            let mut rng = StdRng::seed_from_u64(77);
            let players: Set<PeerPubkey> =
                Set::from_iter_dedup(self.peers.iter().map(|k| k.public_key()));
            let (outcome, share_map) =
                deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                    .expect("deal");
            let shares = self
                .peers
                .iter()
                .map(|p| share_map.get_value(&p.public_key()).expect("share").clone())
                .collect();
            (outcome, shares)
        }

        /// A finalization whose certificate CARRIES the round's σ: every signer
        /// holds a threshold share and the assembler recovers the seed into the
        /// cert. It is the only shape in which σ ever reaches a follower.
        fn certify_seeded(
            &self,
            epoch: u64,
            outcome: &DkgOutcome,
            shares: &[Share],
            payload: Digest,
        ) -> Finalization<BlsScheme, Digest> {
            let bimap = self.bimap();
            let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
            let seed_ns = seed_namespace(&fluent_namespace(CHAIN_ID));
            let oracle = |share: Option<Share>| {
                Arc::new(DealtOracle {
                    sharing: outcome.public().clone(),
                    share,
                    namespace: seed_ns.clone(),
                }) as Arc<dyn SeedOracle>
            };
            let round = Round::new(Epoch::new(epoch), View::new(1));
            let proposal = Proposal::new(round, View::new(0), payload);
            let finalizes: Vec<_> = self
                .bls
                .iter()
                .zip(shares)
                .take(3)
                .map(|(kp, share)| {
                    let signer = build_signer(
                        &ns,
                        bimap.clone(),
                        kp,
                        epoch,
                        Some(oracle(Some(share.clone()))),
                    )
                    .expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, epoch, Some(oracle(None))),
                finalizes.iter(),
                &Sequential,
            )
            .expect("quorum + recovered seed")
        }

        fn certify(&self, epoch: u64, payload: Digest) -> Finalization<BlsScheme, Digest> {
            let bimap = self.bimap();
            let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
            let round = Round::new(Epoch::new(epoch), View::new(1));
            let proposal = Proposal::new(round, View::new(0), payload);
            let finalizes: Vec<_> = self
                .bls
                .iter()
                .take(3)
                .map(|kp| {
                    let signer = build_signer(&ns, bimap.clone(), kp, epoch, None).expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, epoch, None),
                finalizes.iter(),
                &Sequential,
            )
            .expect("quorum")
        }
    }

    /// The same proposal carrying a group key the caller chose, so the artifact
    /// the follower adopts is the key its certificates' σ was formed under.
    fn proposal_keyed(epoch: u64, group_key: DkgOutcome) -> DkgProposal {
        DkgProposal {
            target_epoch: epoch,
            logs: (0..N as u8)
                .map(|i| (i, B256::repeat_byte(0x40 + i)))
                .collect(),
            group_key,
            confirms: Vec::new(),
        }
    }

    /// A recording upstream. `served` is swappable so one test can watch what the
    /// follower does with two different answers to the same question.
    #[derive(Clone, Default)]
    struct Upstream {
        served: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
        calls: Arc<AtomicUsize>,
        /// Set for as long as the caller is inside the verdict. The fetch
        /// asserting on it is what makes "never inline" a real observation rather
        /// than a claim about a signature.
        forbidden: Arc<AtomicBool>,
    }

    impl Upstream {
        fn fetch(&self) -> ArtifactFetch {
            let me = self.clone();
            Arc::new(move |_epoch: u64| {
                assert!(
                    !me.forbidden.load(Ordering::SeqCst),
                    "the fetch ran on the caller's task — the verdict must only file and send"
                );
                me.calls.fetch_add(1, Ordering::SeqCst);
                let served = me.served.lock().expect("served").clone();
                let forbidden = me.forbidden.clone();
                Box::pin(async move {
                    assert!(
                        !forbidden.load(Ordering::SeqCst),
                        "the fetch future was polled on the caller's task"
                    );
                    served
                }) as BoxFuture<'static, _>
            })
        }
    }

    fn config(c: &Committee, up: &Upstream) -> ResolvedFollowerInputs {
        let committee = c.epoch_committee(TARGET);
        ResolvedFollowerInputs {
            chain_id: CHAIN_ID,
            committees: Arc::new(move |epoch: u64| (epoch == TARGET).then(|| committee.clone())),
            // Only TARGET minted; every epoch above it carries TARGET's key.
            changed: Arc::new(|epoch: u64| Some(epoch == TARGET)),
            fetch: up.fetch(),
        }
    }

    /// Hand the follower a σ for `epoch` and return the verdict.
    ///
    /// THIS IS HOW A WANT IS RAISED since row 5.2 retired `observe_cert`: a
    /// `Pending` verdict — a σ this node cannot check — IS the statement "I need
    /// `PK_epoch` for this epoch". Production raises it from the same two cert
    /// doors, so a test that drove a want any other way would be driving a fixture.
    fn hand_seed(
        fb: &FollowerBeacon,
        c: &Committee,
        epoch: u64,
        outcome: &DkgOutcome,
        shares: &[Share],
    ) -> (Observed, Round) {
        let cert = c.certify_seeded(epoch, outcome, shares, Digest(B256::repeat_byte(0xcc)));
        let round = cert.proposal.round;
        let observed = fb
            .randomness
            .observe_certificate(ObservedCertificate::Finalization(round, &cert));
        (observed, round)
    }

    /// Let the fetch task run to the point where `pred` holds. Bounded so a
    /// regression fails the assertion below instead of hanging the suite.
    async fn settle(ctx: &deterministic::Context, pred: impl Fn() -> bool) {
        for _ in 0..64 {
            if pred() {
                return;
            }
            ctx.sleep(Duration::from_millis(1)).await;
        }
    }

    /// A follower must not trust its upstream for the key any more than for a
    /// certificate. An artifact carrying a quorum of the WRONG committee is
    /// refused, nothing is stored, and the epoch stays unpinned — i.e. its certs
    /// keep taking vote-only admission rather than being verified against a key
    /// the upstream chose.
    ///
    /// The forged bytes are asserted to DECODE first, so the refusal is proven to
    /// come from the committee check and not from the codec.
    #[test]
    fn an_artifact_certified_by_the_wrong_committee_is_refused() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let foreign = committee(2);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let forged: AgreedArtifact = (body.clone(), foreign.certify(TARGET, body.digest()));
            let bytes = forged.encode().to_vec();
            assert!(
                decode_artifact(&bytes).is_ok(),
                "the forgery must survive the codec, or this tests the decoder"
            );

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(bytes);
            let fb = build_resolved(&ctx, config(&c, &up));

            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending,
                "the want rides the `Pending` verdict, so this node must be keyless here"
            );
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;

            assert!(
                up.calls.load(Ordering::SeqCst) > 0,
                "the fetch must have run"
            );
            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "a refused artifact must leave the epoch KEYLESS"
            );
            assert!(
                fb.randomness.artifact_bytes(TARGET).is_none(),
                "a refused artifact must not be stored, let alone re-served to a tier-2 follower"
            );
        });
    }

    /// The positive half: a genuine artifact is adopted, the epoch's key then
    /// resolves, and it resolves WITHOUT a fetch — the certificate path stays
    /// network-free after the one off-path delivery.
    #[test]
    fn a_genuine_artifact_is_adopted_and_then_resolves_without_a_fetch() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));
            let expected = *crate::beacon::outcome::group_public_key(&genuine.0.group_key);

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));

            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "nothing is held before the first delivery"
            );
            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            settle(&ctx, || fb.randomness.artifact_bytes(TARGET).is_some()).await;

            let after_adoption = up.calls.load(Ordering::SeqCst);
            assert_eq!(after_adoption, 1, "exactly one delivery");
            assert!(fb.randomness.ensure_key(TARGET, PinEffort::Local).await);
            // The VALUE, read off the adopted artifact itself: `ensure_key` reports
            // only that a key resolved, so without this the test would pass on a
            // wrong one.
            assert_eq!(
                *crate::beacon::outcome::group_public_key(
                    &decode_artifact(&fb.randomness.artifact_bytes(TARGET).expect("adopted"))
                        .expect("decodes")
                        .0
                        .group_key
                ),
                expected,
                "the adopted PK_epoch is the genuine one"
            );
            // A carried (stable) epoch above the mint resolves off the SAME
            // artifact through the dkgQual walk — no second delivery.
            assert!(
                fb.randomness.ensure_key(TARGET + 3, PinEffort::Local).await,
                "a stable epoch carries the minting epoch's key"
            );
            // And the trigger stands down: with the key held the same certificate
            // is `Recorded` instead of `Pending`, so no want is raised at all.
            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Recorded,
                "a checkable σ must not raise a key want"
            );
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > after_adoption).await;
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                after_adoption,
                "ensure_key and the verdict must not spend the network once the key is held"
            );
            assert!(
                fb.randomness.artifact_bytes(TARGET).is_some(),
                "the adopted artifact is servable to a tier-2 follower"
            );
        });
    }

    /// The verdict runs once per verified certificate (~1/s) on the task that
    /// drains the cert stream. Raising a want must be a non-blocking send and
    /// nothing else: the fetch, the decode and the verify all belong to the
    /// background task. The `forbidden` flag is what proves it — the fetch panics
    /// if it is reached while the caller is still inside the verdict.
    #[test]
    fn the_key_want_neither_blocks_nor_fetches_inline() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));

            up.forbidden.store(true, Ordering::SeqCst);
            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                0,
                "the verdict returned having already fetched — it is on the hot path"
            );
            up.forbidden.store(false, Ordering::SeqCst);

            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;
            assert_eq!(
                up.calls.load(Ordering::SeqCst),
                1,
                "the work happens on the background task, and it does happen"
            );
        });
    }

    /// An upstream that serves nothing — no artifact, or a server too old to know
    /// the method, which reach here identically as `None` — must leave the
    /// follower exactly where it was: unpinned, still asking, and with nothing
    /// stored. It is never a data fault and never poisons the epoch.
    #[test]
    fn an_upstream_with_no_artifact_leaves_the_epoch_unpinned_and_retryable() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let up = Upstream::default();
            let fb = build_resolved(&ctx, config(&c, &up));

            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            settle(&ctx, || up.calls.load(Ordering::SeqCst) > 0).await;

            assert_eq!(up.calls.load(Ordering::SeqCst), 1);
            assert!(
                !fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "no artifact means no key — never a wrong one"
            );
            assert!(fb.randomness.artifact_bytes(TARGET).is_none());

            // The miss is not memoised: once the upstream has it, the very next
            // want adopts it. (The per-epoch throttle bounds HOW OFTEN, and it is
            // the only thing between these two wants.)
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            ctx.sleep(crate::beacon::artifact::PULL_MIN_INTERVAL).await;
            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            settle(&ctx, || fb.randomness.artifact_bytes(TARGET).is_some()).await;
            assert!(
                fb.randomness.ensure_key(TARGET, PinEffort::Local).await,
                "a miss must not be terminal"
            );
        });
    }

    /// σ reaches this node class on the certificate and nowhere else — a follower
    /// forms no round and has no by-round transport — and its executor derives a
    /// beacon-active block's `prev_randao` from that σ alone. So a checked σ has
    /// to be filed and served back, and the seed edge has to be the index's
    /// rather than the `idle` handle nothing ever fires.
    ///
    /// The key arrives through the KEYLESS want first, because that is the only
    /// route this node class has to one. The σ this test then files belongs to a
    /// CARRIED epoch above the mint, so it is a round the keyless priming never
    /// touched and the filing is the keyed path's own.
    #[test]
    fn a_verified_certificates_seed_is_filed_and_served() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));

            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            settle(&ctx, || fb.randomness.artifact_bytes(TARGET).is_some()).await;
            assert!(fb.randomness.ensure_key(TARGET, PinEffort::Local).await);

            const CARRIED: u64 = TARGET + 3;
            let cert =
                c.certify_seeded(CARRIED, &outcome, &shares, Digest(B256::repeat_byte(0xcc)));
            let round = cert.proposal.round;
            let sigma = cert
                .certificate
                .seed()
                .expect("a beacon-active certificate carries the round seed");
            assert!(
                fb.randomness.seed(round).is_none(),
                "the priming must not have filed this round"
            );

            let mut edge = fb.randomness.subscribe();
            assert_eq!(
                fb.randomness
                    .observe_certificate(ObservedCertificate::Finalization(round, &cert)),
                Observed::Recorded,
                "a σ checkable under the carried key is filed on the spot"
            );

            assert_eq!(
                fb.randomness.seed(round).map(|s| s.signature),
                Some(sigma),
                "the σ the certificate carried is what a later derive reads back"
            );
            let woken = tokio::select! {
                event = edge.recv() => matches!(event, Ok(BeaconEvent::SeedRecorded)),
                _ = ctx.sleep(Duration::from_millis(10)) => false,
            };
            assert!(
                woken,
                "a held tip waits on the seed wake-up; an unfired one parks it forever"
            );
        });
    }

    /// The keyless window is the ORDINARY state here: a follower obtains
    /// `PK_epoch` only by fetching the epoch's artifact, so σ routinely lands
    /// first. Held (`Pending`) rather than dropped, and re-checked when the key
    /// turns up — an unwired settle would discard most of what the cert doors
    /// file, in silence.
    ///
    /// The artifact is SERVABLE from the start and the follower still has no key
    /// when it judges the σ, because nothing fetches until a want exists and the
    /// `Pending` verdict is what raises the first one. The two assertions between
    /// the verdict and the first await are therefore taken with the key provably
    /// absent.
    #[test]
    fn a_seed_that_arrives_before_the_key_is_held_and_then_promoted() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));
            assert!(
                fb.randomness.artifact_bytes(TARGET).is_none(),
                "nothing is fetched before a want exists"
            );

            let cert = c.certify_seeded(TARGET, &outcome, &shares, Digest(B256::repeat_byte(0xcc)));
            let round = cert.proposal.round;
            let sigma = cert
                .certificate
                .seed()
                .expect("a beacon-active certificate carries the round seed");
            assert_eq!(
                fb.randomness
                    .observe_certificate(ObservedCertificate::Finalization(round, &cert)),
                Observed::Pending
            );
            assert!(
                fb.randomness.seed(round).is_none(),
                "an unchecked σ must never be served"
            );

            // The want the `Pending` above raised is the only thing that reaches
            // the network here, and the settle on the artifact's own edge is what
            // releases the held σ.
            settle(&ctx, || fb.randomness.seed(round).is_some()).await;
            // Only a σ that was HELD can be served now: the capture above ran
            // once, and nothing re-delivers it.
            assert_eq!(
                fb.randomness.seed(round).map(|s| s.signature),
                Some(sigma),
                "the key landing must release what the keyless window held"
            );
        });
    }

    /// ONE ERROR LINE PER EPOCH, and the class that had none. A refusal is judged
    /// PER CERTIFICATE — a follower takes one a second — so the unlatched
    /// `first_seed_refusal` this class used to carry printed one ERROR a second for
    /// the life of the epoch, on the node class where a forged upstream is the
    /// whole threat model. The VERDICT is unchanged (`Refused` every time, a σ that
    /// fails an attested key is a witness every time); only the line is bounded,
    /// which is the rule the deleted `keys.rs::reported_invalid_seed` carried and
    /// row 5.2 moved here.
    ///
    /// The forgery is a REAL σ of a neighbouring round under the SAME key, spliced
    /// into this round's certificate: a decodable curve point that verifies for
    /// nothing here, which is what the upstream forger of R-008 serves. It is
    /// asserted to differ from the genuine σ first, so a green run cannot rest on a
    /// splice that did not happen.
    ///
    /// RED BEFORE THE FIX on the third assertion: the old body was
    /// `fn first_seed_refusal(&self, _epoch: u64) -> bool { true }`, so the latch
    /// was never consumed and every certificate printed.
    #[test]
    fn the_refusal_line_is_latched_once_per_epoch() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));

            // The key first: an `Invalid` verdict needs a resolvable key, or the σ
            // is merely `Pending` and no line is printed at all.
            assert_eq!(
                hand_seed(&fb, &c, TARGET, &outcome, &shares).0,
                Observed::Pending
            );
            settle(&ctx, || fb.randomness.artifact_bytes(TARGET).is_some()).await;
            assert!(fb.randomness.ensure_key(TARGET, PinEffort::Local).await);

            // A genuine σ of the NEXT round, spliced into this round's certificate.
            let payload = Digest(B256::repeat_byte(0xcc));
            let mut forged = c.certify_seeded(TARGET + 1, &outcome, &shares, payload);
            let round = forged.proposal.round;
            let neighbour = c
                .certify_seeded(TARGET + 2, &outcome, &shares, payload)
                .certificate
                .seed;
            assert_ne!(
                forged.certificate.seed, neighbour,
                "the splice must change the σ, or this test refuses nothing"
            );
            forged.certificate.seed = neighbour;

            assert_eq!(
                fb.randomness
                    .observe_certificate(ObservedCertificate::Finalization(round, &forged)),
                Observed::Refused,
                "a σ that fails an attested key is refused"
            );
            assert!(
                !Randomness::first_seed_refusal(fb.provider.as_ref(), round.epoch().get()),
                "the first refusal must have consumed this epoch's latch"
            );
            assert!(
                Randomness::first_seed_refusal(fb.provider.as_ref(), round.epoch().get() + 5),
                "and the latch is per epoch, not global"
            );
            // The verdict itself does not change with the latch: a second forged
            // certificate of the same epoch is still refused, and still not served.
            assert_eq!(
                fb.randomness
                    .observe_certificate(ObservedCertificate::Finalization(round, &forged)),
                Observed::Refused
            );
            assert!(
                fb.randomness.seed(round).is_none(),
                "a refused σ is never served"
            );
            // THE COUNT IS NOT LATCHED (review C-11). Bounding the LINE is only
            // legitimate while the refusal stays countable: two certificates were
            // refused above, and both must show.
            assert_eq!(
                fb.provider.metrics().seed_verify_invalid.get(),
                2,
                "the latch bounds the line, never the count"
            );
        });
    }

    /// THE LATE HALF OF Д-3 ON THE FOLLOWER (review C-06). A forged σ that arrives
    /// BEFORE the epoch key is `Pending` — the ordinary case on this class, since a
    /// follower obtains `PK_epoch` only by fetching the artifact — so
    /// `observe_certificate` has already answered the inlet by the time the refusal
    /// is reached. Without a channel the verdict reaches nobody and a lying
    /// upstream pays nothing; the plane has had one since row 5.0 and this class
    /// had none at all.
    ///
    /// FALSIFIER (one line): delete `self.faults.report(DataFault { epoch, refused
    /// });` in `FollowerRandomness::settle_pending`. The settle still drops the σ
    /// and still writes its ERROR line, and this test goes red on the `try_recv`.
    #[test]
    fn a_late_refusal_reaches_the_followers_fault_consumer() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let (outcome, shares) = c.deal();
            let body = proposal_keyed(TARGET, outcome.clone());
            let genuine: AgreedArtifact = (body.clone(), c.certify(TARGET, body.digest()));

            let up = Upstream::default();
            *up.served.lock().expect("served") = Some(genuine.encode().to_vec());
            let fb = build_resolved(&ctx, config(&c, &up));

            // The consumer arms the channel; nothing is queued before this.
            let mut faults = fb
                .randomness
                .faults()
                .expect("the follower files late verdicts");
            assert!(
                fb.randomness.faults().is_none(),
                "and at most one consumer can take it"
            );

            // A genuine σ of a neighbouring round spliced into this one's
            // certificate — the upstream forgery of R-008 — handed over while this
            // node is still keyless for the epoch.
            let payload = Digest(B256::repeat_byte(0xcc));
            let mut forged = c.certify_seeded(TARGET + 1, &outcome, &shares, payload);
            let round = forged.proposal.round;
            let neighbour = c
                .certify_seeded(TARGET + 2, &outcome, &shares, payload)
                .certificate
                .seed;
            assert_ne!(
                forged.certificate.seed, neighbour,
                "the splice must change the σ, or this test refuses nothing"
            );
            forged.certificate.seed = neighbour;
            assert_eq!(
                fb.randomness
                    .observe_certificate(ObservedCertificate::Finalization(round, &forged)),
                Observed::Pending,
                "keyless, so the verdict at the door can only be `Pending`"
            );
            assert!(
                faults.try_recv().is_err(),
                "and nothing is filed while the verdict is still open"
            );

            // The want raised above pulls the artifact; its landing settles the
            // held σ, and THAT is where the refusal is finally reached.
            settle(&ctx, || fb.randomness.artifact_bytes(TARGET).is_some()).await;
            let epoch = round.epoch().get();
            settle(&ctx, || {
                fb.randomness.seed(round).is_none() && {
                    Randomness::oracle_for(fb.provider.as_ref(), epoch).is_some()
                }
            })
            .await;
            assert_eq!(
                faults.try_recv(),
                Ok(DataFault { epoch, refused: 1 }),
                "the late refusal has to reach the consumer that can act on it"
            );
        });
    }

    /// The beacon-active rule, which binds every implementation: an ORACLE tells
    /// `verify_certificate` that the epoch is beacon-active, so one on a
    /// pre-beacon epoch rejects every LEGAL seedless certificate there.
    #[test]
    fn a_pre_beacon_epoch_gets_no_oracle_and_no_key() {
        let runner = deterministic::Runner::default();
        runner.start(|ctx| async move {
            let c = committee(1);
            let up = Upstream::default();
            let fb = build_resolved(&ctx, config(&c, &up));
            for epoch in 0..super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH {
                assert!(
                    fb.randomness.oracle_for(epoch).is_none(),
                    "epoch {epoch} predates the beacon: an oracle there would reject \
                     every legal seedless certificate"
                );
                assert!(!fb.randomness.ensure_key(epoch, PinEffort::Local).await);
                assert!(!fb.randomness.mandatory_at(epoch));
            }
        });
    }
}

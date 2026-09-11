//! The beacon's node-facing seam: one [`build`] call, one `Arc<dyn Beacon>`, one
//! [`Tasks`].
//!
//! How the epoch key is agreed, where the artifact is stored, how a share is
//! derived and how a peer is served all stay behind it. What crosses it is two
//! task handles and the staking reads that necessarily run the other way — they
//! read the reth state this crate has no access to, and they arrive as ONE
//! [`CommitteeReads`] so every one of them lands on the same cursor.

use alloy_primitives::B256;
use commonware_consensus::types::Epoch;
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
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError, RwLock},
    time::Duration,
};
use tokio::sync::{mpsc, watch, Notify};
use tracing::{error, info, warn};

use crate::{
    beacon::{
        actor::{
            AgreedOutcomeAt, CeremonyStore, CommitteeFor, CommitteePairFor, DkgActor, DkgLogIndex,
            PinnedRequest, PullArtifact,
        },
        artifact::{
            self, decode_artifact, restart_replay, ArtifactBridge, ArtifactPull, ArtifactStore,
            CommitteeSource, PullAnswer,
        },
        carry::{frozen_dkg_qual, DkgQualFor},
        dkg_agree::{AgreedArtifact, ConfirmPool},
        dkg_engine::{
            spawn_agreement_launcher, AgreementMuxes, AgreementPlaneConfig, AgreementTimeouts,
        },
        keys::{AgreedKeys, BeaconKeys, KeySource},
        log_resolver::{BeaconFetchHandler, BeaconFetchKey, LogFetcher, LogHandler, LogMessage},
        metrics::BeaconMetrics,
        outcome::group_public_key,
        share_state::{self, ShareState},
        surface::{LiveBeacon, LiveBeaconConfig},
        Beacon, BeaconEvent, DataFault,
    },
    dpos::{ARTIFACT_JOURNAL_PARTITION, KEY_JOURNAL_PARTITION, SEED_JOURNAL_PARTITION},
    outer::SharedMux,
};

/// The resolver mailbox both beacon subjects — the `{epoch, dealer}` dealer log
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
/// where `read_at` fell back to the GENESIS hash and the write-once memo in
/// [`super::carry::frozen_dkg_qual`] would have frozen `false` for that epoch
/// for the life of the process. The single implementation of this trait is now
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
    /// `at`. The freeze/memo rule that turns it into the carry-forward arbiter is
    /// [`super::carry::frozen_dkg_qual`]'s and stays on this side of the boundary.
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

/// The two handles the node owes the beacon, and the one receiver row 5.4 will
/// take away.
///
/// Two, not eight: a beacon child dying is one fact to the node ("a subsystem
/// died, take the node down"), and which child it was belongs in the log line the
/// supervisor writes, not in the node's supervision list.
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
    /// Supervisor handles of the agreement instances the launcher starts, for
    /// `epoch_manager` to adopt so they prune on the engine cutoff.
    ///
    /// STILL OUT HERE, and it is the one part of the front door PLAN row 5.0 does
    /// not close: the band sweep that consumes these lives in
    /// `epoch_manager::prune_agreements`, and moving it is row 5.4's named work
    /// (it carries the abort-then-join semantics and the SafetyHalt latch, which
    /// are not a code move). Bringing the receiver in without the sweep would
    /// leave the instances unpruned.
    pub agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)>,
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
    /// The `PK_epoch` ladder's two artifact rungs. Both answer the same question
    /// of the same object — the minting epoch's agreed artifact — and differ only
    /// in where they look: `held` reads the local store (memory, then the durable
    /// mirror), `pull` spends one bounded peer fetch on top of it. Splitting them
    /// is what lets the vote-path caller take the cheap one and the off-path
    /// repair sweep take both.
    held_keys: AgreedKeys,
    pull_keys: AgreedKeys,
    /// The `DkgActor`'s live-epoch artifact pull — a SECOND consumer of the same
    /// [`ArtifactPull`] the key ladder's `pull` rung uses, in the actor's
    /// fire-and-forget shape. Built here because this is the only place the pull
    /// and the resolver mailbox exist together.
    pull_artifact: PullArtifact,
}

/// Open the beacon recovery seam: the `commonware_resolver::p2p` engine carrying
/// both beacon subjects, and the two artifact-backed key rungs built over it.
///
/// The engine is built HERE rather than at the node's plane site because the two
/// rungs are the reason it exists: `pull_keys` is a `fetch` on the very mailbox
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
    dkg_qual: DkgQualFor,
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
            me: Some(me),
            initial: RESOLVER_INITIAL,
            timeout: RESOLVER_TIMEOUT,
            fetch_retry_timeout: RESOLVER_RETRY,
            priority_requests: false,
            priority_responses: false,
        },
    );
    let resolver_handle = engine.start(channel);

    let held_keys = AgreedKeys::new(
        Arc::new(move |epoch: u64| {
            let key = store.get(epoch).map(|a| *group_public_key(&a.0.group_key));
            Box::pin(async move { key }) as BoxFuture<'static, _>
        }),
        dkg_qual.clone(),
    );
    // ONE pull for both consumers, so they share the per-epoch throttle that bounds
    // how often this node asks its peers for the same artifact.
    let pull = ArtifactPull::new(context.with_label("artifact_pull"), bridge);
    let pull_keys = {
        let pull = pull.clone();
        let mailbox = mailbox.clone();
        AgreedKeys::new(
            Arc::new(move |epoch: u64| {
                let pull = pull.clone();
                let mut resolver = mailbox.clone();
                Box::pin(async move {
                    match pull.pull(&mut resolver, epoch).await {
                        Some(PullAnswer::Have(a)) => Some(*group_public_key(&a.0.group_key)),
                        // `NotYet` and an exhausted walk are the same answer to
                        // this caller: nobody can give it the key right now, so
                        // the rung yields and the caller stays unpinned.
                        _ => None,
                    }
                }) as BoxFuture<'static, _>
            }),
            dkg_qual,
        )
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
        held_keys,
        pull_keys,
        pull_artifact,
    }
}

/// Spawn the agreement write-back's middle hop.
///
/// It does two things and neither belongs to the actor. It publishes the agreed
/// key at [`KeySource::Agreed`], which is what lets W1 stand down for the epoch
/// and still leaves ladder rung 1 answered for `repair_keyless_schemes`. Then it
/// hands the artifact to the `DkgActor`, where its dealer-log set becomes the
/// pinned set the existing finalize rails run over — the write-back proper.
///
/// Spawned separately from the rest of the plane because `beacon_keys` does not
/// exist until the consensus layer has launched, which is after the plane is
/// built.
fn spawn_write_back<E>(
    context: &E,
    mut agreed_rx: mpsc::Receiver<AgreedArtifact>,
    adopt_tx: mpsc::Sender<AgreedArtifact>,
    beacon_keys: BeaconKeys,
) -> Handle<()>
where
    E: Metrics + Spawner,
{
    context
        .with_label("agreement_write_back")
        .spawn(move |_| async move {
            while let Some(artifact) = agreed_rx.recv().await {
                let epoch = artifact.0.target_epoch;
                let pk = *group_public_key(&artifact.0.group_key);
                info!(
                    epoch,
                    "beacon: epoch-key agreement artifact adopted — publishing PK_epoch"
                );
                beacon_keys.set_pk(epoch, pk, KeySource::Agreed);
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
    /// `active_registry ∪ committee[E]`, which is the `latest.primary`
    /// reachability both the dealer-log resolver and the body engine need.
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
    /// [`crate::beacon::agreement_partition`]) and the key / seed / artifact
    /// journals (`{prefix}` ‖ [`KEY_JOURNAL_PARTITION`] etc., see
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
    let committee_pair_for: CommitteePairFor = {
        let reads = committees.clone();
        Arc::new(move |target: u64| reads.committee_pair(target))
    };
    let committee_source: CommitteeSource = {
        let reads = committees.clone();
        Arc::new(move |epoch: u64| reads.committee_bls(epoch, reads.read_at()?))
    };
    let dkg_qual_for = frozen_dkg_qual(
        {
            let reads = committees.clone();
            Arc::new(move || reads.read_at())
        },
        {
            let reads = committees.clone();
            Arc::new(move |epoch: u64, at: B256| reads.dkg_qual(epoch, at))
        },
    );
    let me = peer_keypair.public_key();
    let share_state = match share_seal_key {
        Some(key) => ShareState::Encrypted(key),
        None => ShareState::Plaintext,
    };

    // Shared live-DKG store, reloaded from the share dir ONCE.
    let ceremony_store: CeremonyStore = Arc::new(RwLock::new(BTreeMap::new()));
    let share_notify = Arc::new(Notify::new());
    // The artifacts ride out of the share files here but land further down, once
    // the artifact store is open (it does not exist yet at this point).
    let mut reloaded_artifacts: Vec<(u64, Vec<u8>)> = Vec::new();
    let reloaded = share_state::load_all(&share_dir, &share_state);
    if !reloaded.is_empty() {
        if let Ok(mut store) = ceremony_store.write() {
            for (epoch, output, share, artifact) in reloaded {
                info!(
                    epoch,
                    artifact = artifact.is_some(),
                    "beacon: reloaded persisted live-DKG share from disk"
                );
                if let Some(bytes) = artifact {
                    reloaded_artifacts.push((epoch, bytes));
                }
                store.insert(epoch, (output, share));
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
    let (agreement_intake_tx, agreement_intake) =
        mpsc::channel::<(Epoch, Handle<()>)>(EDGE_MAILBOX);
    // The write-back's two ends. They are NOT joined here — the hop between them
    // publishes the agreed key into the consensus layer's `BeaconKeys`, which does
    // not exist until that layer launches.
    // The cross-epoch `epoch → PK_epoch` store. Opened HERE, not at the layer
    // launch, and that reordering is what lets the agreement write-back be armed
    // in place below instead of being handed out unjoined for the node to arm
    // later (`BeaconWriteBack`, deleted with this change).
    let (beacon_keys, key_writer) = super::key_journal::open(
        context.with_label("key_journal"),
        context.with_label("key_journal_writer"),
        &journal_partition(&partition_prefix, KEY_JOURNAL_PARTITION),
    )
    .await?;

    // Shared `round → recovered seed` map for the Stage-2 beacon certify gate
    // (`crate::beacon::certify`). The spec-exec reporter writes it (it already
    // recovers the seed per notarization); each per-epoch `BeaconCertify`
    // wrapper reads it, and the executor holds a clone for the speculative
    // seed re-canonicalisation. Cross-epoch singleton — created BEFORE the
    // executor (its first consumer below).
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
        super::certify::SEED_RETENTION,
    )
    .await?;

    let (agreed_tx, agreed_rx) = mpsc::channel::<AgreedArtifact>(EDGE_MAILBOX);
    let (adopt_tx, artifacts_rx) = mpsc::channel::<AgreedArtifact>(EDGE_MAILBOX);

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
    // Refill from the share files, AFTER the journal's own rehydration. `insert` is
    // first-wins, so the journal's copy always stands and this only fills a gap —
    // an epoch whose share was persisted but whose journal record never synced.
    // Not re-verified against `committee[epoch]` here, matching the journal replay:
    // both read a 0600 file this node wrote itself, and refusing to start over an
    // unreadable one would forfeit the very key the store exists to serve.
    for (epoch, bytes) in reloaded_artifacts {
        match decode_artifact(&bytes) {
            Ok(artifact) => {
                if artifact_store.insert(epoch, artifact) {
                    info!(
                        epoch,
                        "beacon: reloaded the agreed artifact from the share file"
                    );
                }
            }
            Err(e) => warn!(
                epoch,
                ?e,
                "beacon: the share file's agreed artifact does not decode; re-agreeing"
            ),
        }
    }
    // Pick the artifacts this restart owes the `DkgActor`. Nothing else reads the
    // store back INTO the actor, so a member that went down between adopting an
    // artifact and finalizing over it would otherwise wait for a re-agreement its
    // peers have already marked started.
    let held_shares: BTreeSet<u64> = ceremony_store
        .read()
        .map(|shares| shares.keys().copied().collect())
        .unwrap_or_default();
    let replay = restart_replay(&artifact_store, &share_dir, &held_shares);

    let ArtifactSeam {
        resolver_handle,
        logs,
        held_keys,
        pull_keys,
        pull_artifact,
    } = open_artifact_seam(
        context,
        chain_id,
        me.clone(),
        peers.clone(),
        resolver_channel,
        artifact_store.clone(),
        committee_source.clone(),
        dkg_qual_for.clone(),
        agreed_tx.clone(),
        metrics.clone(),
        log_handler,
    );

    // The demote-heal reads the agreed `Output` for an EPOCH out of the artifact
    // store. It used to read the boundary block at `epoch_start(E)`, which was a
    // chicken-and-egg — the heal exists for a member that could not enter `E`, and
    // `E`'s own first block is what such an epoch does not produce.
    let outcome_at: AgreedOutcomeAt = {
        let store = artifact_store.clone();
        Arc::new(move |epoch: u64| {
            let outcome = store.get(epoch).map(|a| a.0.group_key.clone());
            Box::pin(async move { outcome }) as Pin<Box<dyn Future<Output = _> + Send>>
        })
    };

    // One clone for the serving read `consensus_getEpochArtifact` answers from,
    // and one geometry receiver for `can_participate`'s unfrozen answer. Taken
    // here because the store moves into the agreement launcher further down and
    // the actor's spawn wrapper takes the watch.
    let artifact_store_for_serving = artifact_store.clone();
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
            let actor = DkgActor::new(
                dkg_namespace,
                peer_keypair,
                sender,
                receiver,
                Some(logs),
                Some(log_resolver_rx),
                committee_for,
                ceremony_store,
                share_notify,
                activation,
                interval,
                actor_metrics,
                Some(share_dir),
                share_state,
                Some(outcome_at),
            )
            .with_committee_pair(committee_pair_for)
            .with_recorded_logs(recorded)
            .with_share_confirms(confirms)
            .with_pinned_requests(pinned_rx)
            .with_agreement_plane(agreement_request_tx, artifacts_rx)
            .with_artifact_pull(pull_artifact)
            .with_plane_clock(plane_clock);
            actor.run(heights, c).await
        })
    };

    // The epoch-key agreement launcher. It owns everything the `DkgActor` cannot
    // reach — the four mux sub-channel registrations, the staking committee read
    // and the runtime context an instance is spawned on — and turns the actor's
    // dealing-closed edge into a running instance.
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
        },
        AgreementMuxes {
            vote: vote_mux,
            cert: cert_mux,
            resolver: resolver_mux,
            bodies: bodies_mux,
        },
        agreement_request_rx,
        agreed_tx.clone(),
        agreement_intake_tx,
    );

    // The agreement write-back, armed IN PLACE. It used to be handed out
    // unjoined (`BeaconWriteBack`) for the node to arm after the consensus layer
    // had created the key store; the store is created above now, so the two ends
    // meet here and the arm-later dance is gone.
    let write_back_handle = spawn_write_back(context, agreed_rx, adopt_tx, beacon_keys.clone());
    // The replay is pushed AFTER the hop is spawned and not before, because this
    // hop is the channel's only drain: a send issued first would deadlock on a
    // store holding more records than the channel's depth.
    if !replay.is_empty() {
        info!(
            epochs = replay.len(),
            "beacon: replaying locally-stored agreement artifacts into the write-back"
        );
    }
    for artifact in replay {
        let epoch = artifact.0.target_epoch;
        if agreed_tx.send(artifact).await.is_err() {
            warn!(
                epoch,
                "beacon: the agreement write-back is gone; the artifact replay stopped"
            );
            break;
        }
    }

    // Everything randomness-shaped, behind ONE handle. This is the only place
    // where all of its inputs exist at once — the ceremony store, the frozen
    // `dkgQual` arbiter, the key store, the seed store and the two agreement
    // rungs — and none of them crosses back out.
    let namespace = seed_namespace(&fluent_namespace(chain_id));
    let quarantine = seed_store.clone();
    let seed_events = seed_store.events().clone();
    let randomness = LiveBeacon::build(LiveBeaconConfig {
        artifacts: artifact_store_for_serving,
        geometry: geometry_for_probe,
        seeds: seed_store,
        keys: beacon_keys.clone(),
        resolver: super::resolve::beacon_share_resolver(
            ceremony_store.clone(),
            dkg_qual_for.clone(),
            namespace,
            beacon_keys.clone(),
        ),
        ceremony: ceremony_store.clone(),
        dkg_qual: dkg_qual_for.clone(),
        held: Some(held_keys.clone()),
        pull: Some(pull_keys),
        metrics: metrics.clone(),
        chain_id,
    });

    // The promotion trigger for quarantined σ. A seed captured at ingress before
    // its epoch key resolved is held, not dropped — and the event that decides
    // whether it can be served is exactly the one this waits on. The boundary
    // does not DEPEND on it (a leader that misses its witness asks for it on the
    // propose path), but an untriggered quarantine is a value the node holds and
    // can never use.
    //
    // Its OWN edge (`BeaconKeys::subscribe`), never the `Arc` the epoch manager's
    // reconcile arm holds: one `notify_one` shared by two waiters swallows one of
    // them, and both losses are silent — an epoch stuck vote-only, or a σ that
    // never leaves quarantine.
    let promoter_randomness = randomness.clone();
    let promoter_edge = beacon_keys.subscribe();
    let seed_promoter_handle = context
        .with_label("seed_promoter")
        .spawn(move |_| async move {
            loop {
                promoter_edge.notified().await;
                for epoch in quarantine.quarantined_epochs() {
                    let Some(oracle) = Beacon::oracle_for(promoter_randomness.as_ref(), epoch)
                    else {
                        continue;
                    };
                    let (promoted, refused) = quarantine.promote_epoch(epoch, oracle.as_ref());
                    if promoted > 0 || refused > 0 {
                        info!(
                            epoch,
                            promoted, refused, "beacon: re-checked quarantined seeds"
                        );
                    }
                    // The LATE verdict, on the channel that may not lose it. The
                    // ERROR line `promote_epoch` already writes stays: this is the
                    // machine-readable half, and it is dropped on the floor until a
                    // consumer has taken the receiver.
                    if refused > 0 {
                        promoter_randomness.report_fault(DataFault { epoch, refused });
                    }
                }
            }
        });

    // The wake-up bridge. `record_seed` fires its own class from inside the
    // beacon; these two are written from OTHER tasks through a bare `Notify` —
    // `BeaconKeys::set_pk` and the `DkgActor`'s share edge — and `notify_one`
    // wakes exactly ONE waiter, so this task being their SOLE waiter is what lets
    // any number of consumers subscribe without swallowing each other's edges.
    //
    // SUPERVISED: with it dead the executor never learns a key landed and the
    // epoch manager never learns its participation changed, which is a silent
    // stall rather than a visible failure.
    let bridge_handle = {
        // Its OWN key notifier, never the shared `notifier()` handle: the seed
        // promoter waits on a subscription of its own for the same reason.
        let key_edge = beacon_keys.subscribe();
        let participation_edge = share_notify.clone();
        // The SAME publisher `SeedStore::record` fires the seed class into, so the
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
                        () = &mut key => BeaconEvent::KeyAvailable,
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
    if let Some(writer) = key_writer {
        writers.push(("key_journal_writer", writer));
    }
    let tasks = Tasks {
        supervised: spawn_supervisor(
            context,
            vec![
                ("dkg", dkg_handle),
                ("beacon_resolver", resolver_handle),
                ("seed_promoter", seed_promoter_handle),
                ("agreement_launcher", agreement_launcher_handle),
                ("agreement_write_back", write_back_handle),
                ("event_bridge", bridge_handle),
            ],
        ),
        drain: spawn_drain(context, writers),
        agreement_intake,
    };
    Ok((randomness, tasks))
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
            journal_partition("", KEY_JOURNAL_PARTITION),
            "beacon-key-ordinal"
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

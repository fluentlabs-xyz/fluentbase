//! The beacon's node-facing seam: one [`build`] call, one `Arc<dyn Beacon>`, one
//! [`Tasks`].
//!
//! How the epoch key is agreed, where the artifact is stored, how a share is
//! derived and how a peer is served all stay behind it. What crosses it is two
//! task handles and the staking reads that necessarily run the other way, as one
//! [`CommitteeReads`] so every read lands on the same cursor.

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
        Beacon, BeaconEvent, ARTIFACT_JOURNAL_PARTITION, MINT_MEMO_PARTITION,
        SEED_JOURNAL_PARTITION,
    },
    outer::SharedMux,
};

/// The resolver mailbox both beacon subjects — the dealer log and the epoch-key
/// artifact — ride.
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
/// The single cursor is the point: [`Self::read_at`] is resolved once per
/// compound read, which makes [`Self::committee_pair`] structurally unable to
/// straddle a block — it is a provided method over two [`Self::committee`] calls
/// at one hash. Reading the committee and the `dkgQual` bit at different state
/// hashes would let a node see a committee it cannot yet see the qual bit for.
///
/// The single implementation is [`crate::committee::CommitteeReadsFacade`],
/// anchored at `executed_state_hash(ordering_finalized)`, with no genesis
/// fallback: below `commit_height(E)` it answers "not readable", and at or above
/// it the bit is final because the contract writes it in the same call that
/// writes the committee.
pub trait CommitteeReads: Send + Sync {
    /// The state hash every read below is taken at — including the `dkgQual`
    /// leg — or `None` where this node cannot read state yet.
    fn read_at(&self) -> Option<B256>;

    /// `committee[epoch]` as the ordered peer set at `at` — the ceremony roster
    /// and the AM5 idx→pubkey mapping.
    fn committee(&self, epoch: u64, at: B256) -> Option<Set<PeerPubkey>>;

    /// The same frozen committee with its BLS half, projected into the participant
    /// BiMap a certificate is verified under.
    fn committee_bls(&self, epoch: u64, at: B256) -> Option<EpochCommittee>;

    /// One raw on-chain `(dkgQual[epoch], committee[epoch] is committed)` read at
    /// `at`. The freeze/memo rule that turns it into the carry-forward arbiter stays
    /// on this side: [`changed_bit`] drops the `committed` leg (the facade answers it
    /// unconditionally `true`) and [`super::artifact::MintIndex`] caches the decided
    /// bit.
    fn dkg_qual(&self, epoch: u64, at: B256) -> Option<(bool, bool)>;

    /// `committee[target−1]` and `committee[target]` at one state hash — the
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
/// A `Drop` impl rather than the supervisor's exit path, because both exits must
/// abort them and only one runs code: aborting [`Tasks::supervised`] drops the
/// task's future, so anything written after the `select` would never run. The
/// drop releases the seed and key store clones the children hold, so the drain
/// writers see their last sender go and flush.
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
/// Two, not eight: a beacon child dying is one fact to the node, and which child
/// it was belongs in the supervisor's log line. Nothing else crosses out — the
/// agreement instances the launcher starts are owned and swept inside the beacon.
pub struct Tasks {
    /// The beacon's supervisor. Resolving means a supervised child exited, which
    /// is always fatal; aborting it aborts every child.
    pub supervised: Handle<()>,
    /// The journal writers, as one drain. Resolving means every writer this
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
    // The guard is built before the task exists and moved in, not inside the
    // async block: `commonware_runtime::Handle` has no `Drop`, so a supervisor
    // aborted before its first poll would drop the bare vector and leave every
    // child running. Built outside, it is a captured field of the future, so
    // dropping the future (including the `aborted` early-return inside `spawn`)
    // runs `SupervisedChildren::drop`.
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
/// Concurrently, so all of them fit inside the caller's one timeout instead of a
/// stuck device paying for it three times.
///
/// Spawned outside any engine's spawn lineage: commonware aborts a task's
/// descendants, so a writer spawned under the engine would be killed by the
/// `engine.abort()` that is supposed to release it.
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
    /// The resolver engine's start handle — aborted only at process shutdown (it
    /// serves peers' fetches and drives this node's own for the whole process).
    resolver_handle: Handle<()>,
    /// The dealer-log fetch handle the ceremony and every agreement instance take.
    logs: BeaconLogs,
    /// One bounded acquisition of a minting epoch's artifact over this class's
    /// transport — the same [`AcquireArtifact`] the follower is built on, so the
    /// non-member's route and the follower's route are one code path.
    acquire: AcquireMint,
    /// The `DkgActor`'s live-epoch pull — a second consumer of the same
    /// [`ArtifactPull`] `acquire` uses, in the actor's fire-and-forget shape. One
    /// `ArtifactPull` under both shares one per-epoch throttle.
    pull_artifact: PullArtifact,
}

/// Open the beacon recovery seam: the `commonware_resolver::p2p` engine carrying
/// both beacon subjects, and the artifact pull built over it.
///
/// The engine is built here because the pull is the reason it exists:
/// `ArtifactPull::pull` is a `fetch` on the very mailbox `Engine::new` returns,
/// and splitting them across the crate boundary would leave the only caller on
/// the other side.
///
/// `blocker` is an isolated [`NoopBlocker`], not the shared oracle: a
/// `deliver=false` on a bad response must never partition a peer from the
/// consensus channels.
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

    // One pull for both consumers, so they share the per-epoch throttle that
    // bounds how often this node asks its peers for the same artifact.
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
    // The `DkgActor`'s consumer of the same pull. Fire-and-forget: the actor
    // calls this from its height tick and cannot await a bounded fetch, so it
    // spawns and returns. `inflight` is not the rate bound — the throttle is; it
    // stops the spawns from stacking, since `ArtifactPull::throttle` sleeps until
    // the epoch's next slot rather than de-duplicating.
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
/// The verify/store/write-back body lives in [`ArtifactBridge`], the resolver's
/// own `Consumer`, because a `deliver = false` there is what costs a lying peer
/// its standing; moving the check out would move the peer-punishment decision
/// with it.
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
            // `NotYet` and an exhausted walk are the same answer here: nobody can
            // supply the artifact now, so it stays unresolved. The answer's
            // artifact is dropped — `ArtifactBridge::deliver` already verified and
            // filed it, so the store is what to believe.
            match self.pull.pull(&mut resolver, minted_at).await {
                Some(PullAnswer::Have(served)) => {
                    // The store is what to believe, but the answer's own epoch is
                    // logged because "a peer served something" and "a peer served
                    // this epoch" are different readings.
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
/// It publishes the agreed artifact to the `DkgActor`, where its dealer-log set
/// becomes the pinned set the finalize rails run over. The key itself needs no
/// publish: the artifact is already in [`ArtifactStore`], and that store owns the
/// key.
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
            // park, never return. This handle is supervised, where a clean exit
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
    /// This node's p2p identity, moved: the `DkgActor` signs its dealer logs with
    /// it and derives the plane's `me` from it.
    pub peer_keypair: Ed25519PrivateKey,
    pub bls_keypair: ValidatorBlsKeypair,
    /// `<datadir>/beacon` — where per-epoch shares and their artifacts persist.
    pub share_dir: PathBuf,
    /// The at-rest seal key for this node's DKG shares, HKDF-derived from its BLS
    /// key. `Some` ⇒ keystore mode ⇒ shares persist encrypted; `None` ⇒
    /// plaintext-dev.
    pub share_seal_key: Option<ShareSealKey>,
    /// The plane's own oracle. Precondition: its `EpochTransition` tracks
    /// `committee[E−1] ∪ committee[E] ∪ committee[E+1]` as primary, which is the
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
    /// The clock the ceremony's deal/seal geometry runs on: the marshal's
    /// ordering tip as `FluentApp::report(Update::Tip)` publishes it. The node
    /// and testbed create the watch before [`build`] and hand its sender to the
    /// app (`FluentApp::with_beacon_tip`), so the epoch manager and this actor
    /// read one value from one writer, each on its own channel with exactly one
    /// parked receiver.
    ///
    /// A watch, so a consumer that starts late reads the newest tip rather than a
    /// backlog, and nothing is dropped; after a restart the marshal reports its
    /// highest stored finalization at startup, so the clock needs no seed.
    pub clock: watch::Receiver<u64>,
    /// The registered clock pair. The `DkgActor` publishes its half off the
    /// monotone clamp `on_height` keeps over `clock`, so the gauge reports the
    /// clock the ceremony geometry runs on. Arrives from the node crate, where
    /// the registry lives.
    pub plane_clock: crate::sync_metrics::PlaneClock,
    /// The node's fork-safety latch, the one instance the executor and the epoch
    /// manager share. The agreement launcher reads it at every instance spawn and
    /// waits on its 0→1 edge: a halted node starts no agreement instance and
    /// aborts the ones it has. The `DkgActor` does not hold it.
    pub safety_halt: crate::sync_metrics::SafetyHalt,
    /// The plane's frozen `(dpos_activation, epoch_interval)`, as a watch.
    ///
    /// `None` is the
    /// [`GeometryUnfrozen`](super::WithheldReason::GeometryUnfrozen) state, not
    /// an error: `build` neither awaits it nor fails on it, the actor waits for
    /// the first `Some` and starts then. A watch rather than the former one-shot
    /// `Notify`, where a wake-up that raced the freeze left the node with no
    /// `DkgActor` for the life of the process.
    pub geometry: watch::Receiver<Option<(u64, u64)>>,
    /// Prefix of every storage partition the plane opens: the epoch-key
    /// agreement journals and the key, seed and artifact journals. Production
    /// passes `""`; the in-crate testbed a per-node prefix.
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
/// back out. The node keeps the network, the mux brokers, the ordering-tip watch
/// and the staking reads — everything whose dependency runs the other way.
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
        clock,
        plane_clock,
        safety_halt,
        geometry,
        partition_prefix,
    } = cfg;
    // The four closures the internals speak, all projected off one
    // `CommitteeReads`, so every one takes its state hash from the same
    // `read_at`. Two closures called apart can still straddle a block, so the
    // ceremony-start decision uses `committee_pair`, which resolves the cursor
    // once for both epochs.
    let committee_for: CommitteeFor = {
        let reads = committees.clone();
        Arc::new(move |epoch: u64| reads.committee(epoch, reads.read_at()?))
    };
    let committee_source: CommitteeSource = {
        let reads = committees.clone();
        Arc::new(move |epoch: u64| reads.committee_bls(epoch, reads.read_at()?))
    };
    // The frozen `changed` bit, one closure for both node classes — see
    // [`changed_bit`] for why the `committed` leg is dropped rather than guarded.
    let changed: ChangedAt = changed_bit(committees.clone());
    let me = peer_keypair.public_key();
    let share_state = match share_seal_key {
        Some(key) => ShareState::Encrypted(key),
        None => ShareState::Plaintext,
    };

    // Shared live-DKG store, reloaded from the share dir once.
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

    // Beacon counters registered once here; cloned, never re-registered, into the
    // `DkgActor` and each per-epoch signer engine.
    let metrics = BeaconMetrics::default();
    metrics.register(context);

    let dkg_namespace = seed_namespace(&fluent_namespace(chain_id));
    // The epoch-key agreement plane's shared state, created once and handed to
    // both the `DkgActor` and every instance the launcher starts.
    //
    // `confirm_pool` carries the namespace share-confirmations are signed under,
    // so handing the same pool to both sides makes it impossible for them to
    // disagree about it.
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
    // The durable mint memo maps `epoch → minting epoch`, which makes the
    // durable artifact store addressable without a chain read. See
    // `artifact::open_mint_memo` for the one case it does not cover.
    let (mints, mint_writer) = artifact::open_mint_memo(
        context.with_label("mint_memo"),
        context.with_label("mint_memo_writer"),
        &journal_partition(&partition_prefix, MINT_MEMO_PARTITION),
        changed.clone(),
    )
    .await?;

    // The `round → σ` index (`crate::beacon::seed_index`), the one owner of the
    // seed fact. Every door writes it through `Beacon::observe_certificate`; the
    // executor's derive and the epoch manager's boundary base read it
    // synchronously. Cross-epoch singleton, created before the executor.
    //
    // The durable store is opened and replayed here, ahead of every consumer, so
    // no consumer sees a half-rehydrated store; a second handle over the same
    // partition would be a dual-writer.
    //
    // The writer must outlive `engine.abort()`, or the drain that flushes its
    // tail kills it instead. commonware supervision aborts a task's descendants
    // by spawn lineage, and `beacon::build` runs before any engine task exists,
    // so spawning it here keeps it out of every engine's lineage.
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
    // goes straight to the actor, which owns the pull that heals it.
    let (body_lost_tx, body_lost_rx) = mpsc::channel::<u64>(EDGE_MAILBOX);

    // The `LogHandler` bridges the resolver engine's Producer/Consumer to the
    // `DkgActor` run loop, which owns the ceremony state single-threaded.
    let (log_resolver_tx, log_resolver_rx) = mpsc::channel::<LogMessage>(RESOLVER_MAILBOX);
    let log_handler = LogHandler::new(log_resolver_tx);

    // The per-epoch artifact store: RAM for every in-process reader plus a
    // durable mirror. One instance per process; a second handle over this
    // partition would be a dual-writer.
    let (artifact_store, artifact_writer_handle) = artifact::open(
        context.with_label("artifact_store"),
        context.with_label("artifact_store_writer"),
        &journal_partition(&partition_prefix, ARTIFACT_JOURNAL_PARTITION),
    )
    .await?;
    // The store owns the `Conflict` witness on disk as well as in RAM: a second
    // certified value is written into the same directory the actor's `recover`
    // reads, so a restart between the note and the next tick cannot forget it.
    let artifact_store = artifact_store.with_conflict_dir(share_dir.clone());
    // The share file carries no copy of the agreed artifact, so the artifact
    // journal's rehydration is all a restart recovers and an epoch it lost is
    // acquired from peers (`DkgActor::drive_acquisition`). The actor reads the
    // store rather than replaying it: the store owns the epoch's artifact.

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
    // The one owner of `PK_epoch` and the polynomial, assembled where its two
    // halves first exist together: the durable artifact store and the mint memo.
    let key_index = KeyIndex::new(artifact_store.clone(), mints);

    // The actor's read of the artifact store: the held payload and the divergent
    // second value, if the store ever noted one. Reading the boundary block at
    // `epoch_start(E)` would be circular, since the heal exists for a member that
    // could not enter `E`.
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
    // One edge, taken before the store moves into the agreement launcher. Two
    // independent subscriptions would race: both fire on the same artifact insert
    // in either order, so a consumer woken by `KeyAvailable` could re-read the
    // seed index before the promote ran. The bridge settles pending first.
    let bridge_key_edge = artifact_store.subscribe();
    let geometry_for_probe = geometry.clone();

    // The persistent `DkgActor`, spawned once and running for the whole process.
    // It is constructed after the geometry freezes, so it takes plain
    // `(activation, interval)` and never re-reads the chain; the clock watch
    // keeps the newest tip meanwhile.
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
            // Unfrozen is a state, not an error: wait for the first `Some` and
            // start then, rather than reading the geometry once through a one-shot
            // `Notify` that a racing wake-up could miss.
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
            actor.run(clock, c).await
        })
    };

    // The epoch-key agreement launcher. It owns everything the `DkgActor` cannot
    // reach — the four mux sub-channel registrations, the staking committee read
    // and the runtime context an instance is spawned on — turns the actor's
    // dealing-closed edge into a running instance, and owns that instance from
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

    // The agreement write-back, armed in place; the store it needs is created
    // above, so there is no arm-later handoff.
    let write_back_handle = spawn_write_back(context, agreed_rx, adopt_tx);

    // Everything randomness-shaped, behind one handle: the ceremony store, the
    // frozen `dkgQual` arbiter, the key store, the seed store and the agreement
    // rungs all exist at once here and none crosses back out.
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
    // beacon; these two are written from other tasks through a bare `Notify`, and
    // `notify_one` wakes exactly one waiter, so this being their sole waiter is
    // what lets any number of consumers subscribe.
    //
    // Supervised: with it dead the executor never learns a key landed and the
    // epoch manager never learns its participation changed.
    let bridge_handle = {
        let key_edge = bridge_key_edge;
        // The settle rides this task's KEY arm — see `LiveBeacon::settle_pending`
        // for why it is not a task of its own any more.
        let settle = randomness.clone();
        let participation_edge = share_notify.clone();
        // The same publisher `SeedIndex::record` fires the seed class into, so the
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
                            // Before the publish, never after: a consumer re-reads
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

// The `--cert-follow` follower's beacon.
//
// One `Randomness` implementation for both node classes: a follower is a
// configuration of `LiveBeacon` rather than a second implementation.
//
//   * an empty `CeremonyStore`: every material-bound answer goes through
//     `with_material`, which returns `None` when this node holds no share of the
//     mint. `KeyIndex` still resolves the epoch's mint and polynomial from the
//     artifact, so `verify_seed` — the one answer a follower can give — works.
//   * RAM-only stores: no `share_dir` and no durable partition, so a restart
//     loses one fetch per epoch over a link the follower holds open anyway.
//   * `acquire` over this class's own transport, with the key want wired into
//     `hold_seed`: a follower has no epoch manager, so the `Pending` verdict is
//     the only thing that can ask for a key.
//   * no DKG, no agreement plane, no muxes; the `Withheld` verdicts follow from
//     the empty share store. The geometry watch is set to a resolved `(0, 1)` so
//     `share_probe`'s `GeometryUnfrozen` refinement does not fire —
//     `NoUsableShare` is the true story for a node that runs no ceremony.
//
// A follower checks the seed slot as well as the attributable quorum; the artifact
// that lets it resolve the key is delivered by the caller as [`ArtifactFetch`] over
// the follower's cert upstream.
//
// The upstream is trusted for delivery and nothing else: a fetched artifact is
// checked against `committee[epoch]` read from this node's own chain state by the
// same `verify_artifact_for_epoch` a validator's pull seam uses.

/// Depth of the want channel between `Randomness::hold_seed` and the fetch task.
/// Wants are re-issued on every certificate for as long as the epoch stays
/// unresolved, so a full channel costs nothing: the drop is re-asked a second
/// later.
const WANT_MAILBOX: usize = 16;

/// One artifact fetch over the follower's cert upstream, by minting epoch — the
/// bytes half of an acquisition, supplied by the node.
///
/// It is [`artifact::ArtifactBytes`] under this seam's own name. The only
/// supplier is `crate::dpos::CertUpstream::get_epoch_artifact`.
pub type ArtifactFetch = artifact::ArtifactBytes;

/// What [`build_follower`] needs that it cannot build itself. Every field is a
/// capability the node already holds; none of them is beacon state.
///
/// `--cert-follow` has no keys, no muxes and no DKG, so this is a different set
/// rather than a narrowing of [`ValidatorInputs`]: what it has is the artifact
/// upstream, and the artifact half is all a follower's beacon does. There is no
/// `partition_prefix`: the follower is RAM-only by decision.
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
    /// The same object as `randomness`, kept concrete for this file's own tests:
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

    // The same acquisition the non-member validator uses, over this class's
    // transport: throttle, decode, verify against `committee[minted_at]` and
    // file, one body so a lying upstream and a lying peer cannot be two different
    // checks. No write-back hop: this node class has no actor.
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
    // taken inside the loop, and an artifact arrives once per epoch — unlike a want,
    // nothing re-issues it a second later.
    let key_edge = store.subscribe();
    let randomness = LiveBeacon::build(LiveBeaconConfig {
        // RAM-only, like the artifact store above and for the same reason: this
        // path opens no journal partition. What a restart loses is σ the next
        // certificate carries anyway.
        seeds: super::seed_index::SeedIndex::new(),
        keys,
        // The one field that makes this a follower; see the section head.
        ceremony: super::surface::keyless_ceremony(),
        acquire: Some(acquire),
        metrics,
        chain_id: cfg.chain_id,
        artifacts: store,
        // A follower freezes no `(activation, interval)` and never asks to
        // participate, so the only reader of this watch — `share_probe`'s
        // `GeometryUnfrozen` refinement — must not fire: `NoUsableShare` is the
        // true story for a node class that runs no ceremony.
        geometry: watch::channel(Some((0, 1))).1,
    });
    // Before the `Arc` is handed to anything, which is what makes the write-once
    // cell sound.
    randomness.wire_want(want_tx);
    let fetch_handle = {
        // Weak on purpose: the task holds the receiving end of `want_tx`, so an
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

/// The frozen `changed` bit over a [`CommitteeReads`], for both node classes.
///
/// The `committed` leg the raw read carries is dropped rather than guarded: the
/// module answers a record only for an epoch whose committee it read, so
/// `committed` is unconditionally `true`.
pub(super) fn changed_bit(reads: Arc<dyn CommitteeReads>) -> ChangedAt {
    Arc::new(move |epoch: u64| {
        let at = reads.read_at()?;
        reads.dkg_qual(epoch, at).map(|(bit, _committed)| bit)
    })
}

/// The off-path half, and the only place a follower touches the network for a
/// key.
///
/// Sequential by construction, so a slow upstream costs latency and never a
/// fan-out; the per-epoch throttle lives inside `TransportAcquire`. The
/// acquisition is `ensure_key(Thorough)`, the same operation the epoch manager
/// calls on a validator.
///
/// It carries the settle of held σ on a second arm, keyed on the store's own
/// edge rather than this task's fetch result: an epoch can also resolve off an
/// artifact adopted for a different epoch, so a σ waiting on that one would
/// otherwise never be re-checked.
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
                // Before the publish, never after — the plane's bridge takes the
                // same order and for the same reason: a consumer re-reads state on
                // the wake-up, and a σ the landed key has just made servable must
                // already be `Verified` when it does.
                randomness.settle_pending();
                // This task is the sole waiter on the store's own notifier, so it is
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
    // park, never return: this handle is supervised, where a clean exit means "a
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

/// The follower beacon's own tests, kept a module of their own rather than merged
/// into [`tests`]: the two have disjoint fixtures.
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
    /// The epoch whose committee minted the key, so `dkgQual[TARGET]` is set and
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

        /// A real DKG over this committee's peers, so the artifact the follower
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

        /// A finalization whose certificate carries the round's σ: every signer
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
    /// A `Pending` verdict — a σ this node cannot check — is the statement "I need
    /// `PK_epoch` for this epoch", raised from the same cert doors production
    /// uses.
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
    /// certificate. An artifact carrying a quorum of the wrong committee is
    /// refused, nothing is stored, and the epoch stays unpinned — i.e. its certs
    /// keep taking vote-only admission rather than being verified against a key
    /// the upstream chose.
    ///
    /// The forged bytes are asserted to decode first, so the refusal is proven to
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
    /// resolves, and it resolves without a fetch — the certificate path stays
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
            // The value, read off the adopted artifact itself: `ensure_key` reports
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
            // A carried (stable) epoch above the mint resolves off the same
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
            // want adopts it. The per-epoch throttle bounds how often, and it is
            // the only thing between these two wants.
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

    /// σ reaches this node class on the certificate and nowhere else, and its
    /// executor derives a beacon-active block's `prev_randao` from that σ alone.
    /// So a checked σ has to be filed and served back, and the seed edge has to be
    /// the index's rather than an idle handle nothing fires.
    ///
    /// The key arrives through the keyless want first, the only route this class
    /// has to one. The σ filed here belongs to a carried epoch above the mint, so
    /// it is a round the keyless priming never touched.
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

    /// The keyless window is the ordinary state here: a follower obtains
    /// `PK_epoch` only by fetching the artifact, so σ routinely lands first. Held
    /// (`Pending`) rather than dropped, and re-checked when the key turns up.
    ///
    /// The artifact is servable from the start and the follower still has no key
    /// when it judges the σ, because nothing fetches until a want exists.
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
            // Only a σ that was held can be served now: the capture above ran
            // once, and nothing re-delivers it.
            assert_eq!(
                fb.randomness.seed(round).map(|s| s.signature),
                Some(sigma),
                "the key landing must release what the keyless window held"
            );
        });
    }

    /// One error line per epoch, and the class that had none: a refusal is judged
    /// per certificate, so an unlatched line would print once a second for the
    /// life of the epoch. The verdict is unchanged; only the line is bounded.
    ///
    /// The forgery is a real σ of a neighbouring round under the same key,
    /// spliced into this round's certificate, and asserted to differ first so a
    /// green run cannot rest on a splice that did not happen.
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

            // A genuine σ of the next round, spliced into this round's certificate.
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
            // The count is not latched: bounding the line is only legitimate while
            // the refusal stays countable.
            assert_eq!(
                fb.provider.metrics().seed_verify_invalid.get(),
                2,
                "the latch bounds the line, never the count"
            );
        });
    }

    /// A forged σ that arrives before the epoch key is `Pending`, so
    /// `observe_certificate` has already answered by the time the refusal is
    /// reached. Without a channel the verdict reaches nobody and a lying upstream
    /// pays nothing; this class files the late refusal for its fault consumer.
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
            // certificate, handed over while this node is still keyless.
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
            // held σ, and that is where the refusal is finally reached.
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

    /// The beacon-active rule, which binds every implementation: an oracle tells
    /// `verify_certificate` that the epoch is beacon-active, so one on a
    /// pre-beacon epoch rejects every legal seedless certificate there.
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

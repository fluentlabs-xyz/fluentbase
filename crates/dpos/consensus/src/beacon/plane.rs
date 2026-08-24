//! The beacon's node-facing seam: one [`build`] call, one opaque [`Beacon`].
//!
//! How the epoch key is agreed, where the artifact is stored, how a share is
//! derived and how a peer is served all stay behind it. What crosses it is the
//! handles the node has to supervise, the shared state each promoted signer
//! engine reads, and the three staking-state closures that necessarily run the
//! other way — they read the reth state this crate has no access to.

use alloy_primitives::B256;
use commonware_consensus::types::Epoch;
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
use commonware_p2p::{Provider, Receiver, Sender};
use commonware_resolver::p2p::{Config as ResolverConfig, Engine as ResolverEngine};
use commonware_runtime::{BufferPooler, Clock, Handle, Metrics, Spawner, Storage};
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
use tokio::sync::{mpsc, Notify};
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
        carry::{frozen_dkg_qual, DkgQualFor, DkgQualProbe},
        dkg_agree::{AgreedArtifact, ConfirmPool},
        dkg_engine::{
            spawn_agreement_launcher, AgreementMuxes, AgreementPlaneConfig, AgreementTimeouts,
        },
        keys::{AgreedKeys, BeaconKeys, KeySource},
        log_resolver::{BeaconFetchHandler, BeaconFetchKey, LogFetcher, LogHandler, LogMessage},
        metrics::BeaconMetrics,
        outcome::group_public_key,
        share_state::{self, ShareState},
        surface::{PlaneRandomness, PlaneRandomnessConfig},
        Randomness,
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
/// and still leaves ladder rung 1 answered for `repair_unpinned_schemes`. Then it
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
/// The three committee/qual closures stay the node's: they read the reth staking
/// state through the node's own provider and EVM config, which is exactly the
/// dependency this boundary exists to keep out of the consensus crate.
pub struct BeaconConfig<P, Se, Re, XS, XR, HS, HR>
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
    /// `committee[epoch]` as the ordered peer set — the ceremony roster and the
    /// AM5 idx→pubkey mapping.
    pub committee_for: CommitteeFor,
    /// `committee[target−1]` and `committee[target]` at ONE state hash — the
    /// ceremony-start decision's input. See [`CommitteePairFor`].
    pub committee_pair_for: CommitteePairFor,
    /// The SAME frozen committee with its BLS half, projected into the participant
    /// BiMap a certificate is verified under. One closure feeds the artifact seam's
    /// verification and the agreement instance's signer construction, so an
    /// instance and a peer checking its artifact can never disagree about who the
    /// committee is.
    pub committee_source: CommitteeSource,
    /// The finalized state hash every `dkgQual` read is taken at.
    pub dkg_qual_at: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
    /// One raw on-chain `(dkgQual[epoch], committee[epoch] is committed)` read at a
    /// finalized hash. The freeze/memo rule that turns it into the carry-forward
    /// arbiter is [`frozen_dkg_qual`]'s and stays on this side of the boundary —
    /// duplicating it is how one copy drifts.
    pub dkg_qual_probe: DkgQualProbe,
    /// The ORDERING-finalized height clock the ceremony's deal/seal geometry runs
    /// on. Ticks buffered here before the actor starts are drained by `on_height`'s
    /// monotone-max clamp.
    pub heights: mpsc::Receiver<u64>,
    /// The registered clock pair. The `DkgActor` publishes its half off the
    /// monotone clamp that merges every feeder in `heights`, so the gauge reports
    /// the clock the ceremony geometry runs on rather than whichever feeder wrote
    /// last. Arrives from the node crate, where the registry lives.
    pub plane_clock: crate::sync_metrics::PlaneClock,
    /// Resolves the plane's frozen `(dpos_activation, epoch_interval)` — awaited
    /// once, inside the actor's spawn wrapper, so the actor never re-reads the
    /// chain for geometry the plane already froze. `None` ⇒ the geometry is
    /// unreadable and no `DkgActor` starts.
    pub geometry: BoxFuture<'static, Option<(u64, u64)>>,
}

/// Read one stored artifact's wire bytes, or `None` where this node holds none.
///
/// A READ CLOSURE and never the [`ArtifactStore`] itself: [`build`]'s contract is
/// that the store does not cross back out to the node. This is the one capability
/// it grants instead — enough for `consensus_getEpochArtifact` to serve a peer,
/// and nothing else. Serving is safe to expose unauthenticated because the
/// artifact is self-authenticating against `committee[epoch]`, so handing one to
/// anyone leaks nothing a staking read would not.
pub type ArtifactSource = Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>;

/// The always-on beacon: the persistent `DkgActor`, the recovery seam, the
/// epoch-key agreement launcher and the durable artifact store's writer.
pub struct Beacon {
    /// The persistent `DkgActor` task — aborted ONLY at process shutdown.
    pub dkg_handle: Handle<()>,
    /// The dealer-log + artifact recovery resolver engine — aborted ONLY at
    /// process shutdown (it serves peers' fetches and drives this node's own for
    /// the whole process).
    pub resolver_handle: Handle<()>,
    /// The epoch-key agreement launcher: it turns the `DkgActor`'s dealing-closed
    /// edge into a running agreement instance. Aborted ONLY at process shutdown.
    pub agreement_launcher_handle: Handle<()>,
    /// The durable artifact store's writer. A DRAIN handle, not a supervised one:
    /// it returns when the store's last sender drops, which is shutdown.
    pub artifact_writer_handle: Option<Handle<()>>,
    /// Supervisor handles of the agreement instances the launcher starts, for
    /// `epoch_manager` to adopt so they prune on the engine cutoff. Move-only.
    pub agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)>,
    /// The agreement write-back hop, armed at build. A SUPERVISED handle: a clean
    /// exit means a subsystem died.
    pub write_back_handle: Handle<()>,
    /// The durable key journal's writer. A DRAIN handle. `None` ⇒ RAM-only.
    pub key_writer_handle: Option<Handle<()>>,
    /// The durable seed journal's writer. A DRAIN handle, and the one with the
    /// sibling-context requirement — see where it is spawned.
    pub seed_writer_handle: Option<Handle<()>>,
    /// The consensus-facing randomness surface. The ONLY thing the consensus
    /// layer receives from the beacon.
    pub randomness: Arc<dyn Randomness>,
    /// Serve one held artifact's bytes to a peer over `consensus_getEpochArtifact`
    /// — see [`ArtifactSource`] for why this is a closure and not the store.
    pub artifact_bytes: ArtifactSource,
}

/// Build the always-on beacon.
///
/// One call, one opaque result: the ceremony store, the dealer-log index, the
/// artifact store, the confirmation pool, the resolver seam and the agreement
/// launcher are all created here and none of them crosses back out. The node
/// keeps the network, the mux brokers, the finalized-height poller and the three
/// staking-state closures — everything whose dependency runs the other way.
pub async fn build<E, P, Se, Re, XS, XR, HS, HR>(
    context: &E,
    cfg: BeaconConfig<P, Se, Re, XS, XR, HS, HR>,
) -> eyre::Result<Beacon>
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
    let BeaconConfig {
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
        committee_for,
        committee_pair_for,
        committee_source,
        dkg_qual_at,
        dkg_qual_probe,
        heights,
        plane_clock,
        geometry,
    } = cfg;
    let dkg_qual_for = frozen_dkg_qual(dkg_qual_at, dkg_qual_probe);
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
        KEY_JOURNAL_PARTITION,
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
        SEED_JOURNAL_PARTITION,
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
        ARTIFACT_JOURNAL_PARTITION,
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

    // The serving read the node's `consensus_getEpochArtifact` handler answers
    // from. Re-encoded per call rather than kept as bytes beside the artifact:
    // serving is a rare, off-path request (a follower asks once per epoch it
    // lacks), and a second copy of every artifact in RAM to save it would be paid
    // for on every node forever.
    let artifact_bytes: ArtifactSource = {
        let store = artifact_store.clone();
        Arc::new(move |epoch: u64| store.get(epoch).map(|a| artifact::encode_artifact(&a)))
    };

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
            let Some((activation, interval)) = geometry.await else {
                // The geometry future only resolves post-freeze, so this is
                // unreachable for a healthy node. A validator whose geometry is
                // unreadable/unscheduled already failed loud at launch, so this
                // soft path is reached only by a mis-configured non-validator —
                // where staying network/Muxers up (follower connectivity) with
                // no DKG is the intended fail-soft.
                error!("beacon plane: geometry resolved unfrozen; DkgActor not started");
                return;
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
    let randomness = PlaneRandomness::build(PlaneRandomnessConfig {
        seeds: seed_store,
        keys: beacon_keys.clone(),
        verify: Some(super::resolve::BeaconVerify::new(
            super::resolve::group_key_resolver(
                ceremony_store.clone(),
                dkg_qual_for.clone(),
                beacon_keys.clone(),
            ),
            namespace.clone(),
        )),
        resolver: super::resolve::beacon_share_resolver(
            ceremony_store.clone(),
            dkg_qual_for.clone(),
            namespace,
            beacon_keys.clone(),
        ),
        held: Some(held_keys.clone()),
        pull: Some(pull_keys),
        participation: share_notify.clone(),
        metrics: metrics.clone(),
        chain_id,
    });

    Ok(Beacon {
        dkg_handle,
        resolver_handle,
        agreement_launcher_handle,
        write_back_handle,
        artifact_writer_handle,
        key_writer_handle: key_writer,
        seed_writer_handle: Some(seed_writer),
        agreement_intake,
        randomness,
        artifact_bytes,
    })
}

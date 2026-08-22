//! The epoch-key agreement instance: a second, short-lived `simplex` over
//! `committee[target_epoch]`, and the supervisor that ends it.
//!
//! It runs on its own namespace ([`fluentbase_bls::beacon::dkg_namespace`]) and
//! its own sub-channels, agrees exactly one value — the pinned dealer-log set —
//! and is torn down at its own finalization. Five things differ from the ordering
//! plane's engine ([`crate::engine`]) and each of them is load-bearing:
//!
//! 1. a distinct, prefix-free namespace, or an observer of this plane could
//!    assemble an honest validator's two payloads for one `(epoch, view)` into
//!    permissionless equivocation evidence;
//! 2. no `register_scheme` — the shared `EpochSchemeProvider` refuses a
//!    different-committee re-register and prunes by epoch, so writing this
//!    instance's scheme into that slot would drop the ordering engine's;
//! 3. a seedless [`RoundRobin`] elector — the VRF elector this instance exists to
//!    make possible is not available to it;
//! 4. a reporter wired to NEITHER marshal, slasher nor `spec_exec` — agreement
//!    rounds are not consensus rounds, and feeding them to the slasher would
//!    manufacture evidence about rounds the ordering plane never held;
//! 5. its own journal partition, destroyed after the abort.
//!
//! # Lifecycle
//!
//! `simplex::Engine::run` panics if any of its actors finishes and its only clean
//! return is `context.stopped()`, so "decide once and stop" has to come from
//! outside: [`spawn_agreement`] returns ONE handle, the supervisor's, and the
//! supervisor aborts the instance when its reporter delivers a finalization —
//! whichever view that lands in, because a nullified first view is ordinary. The
//! engines are started from the supervisor's OWN context, so an external
//! `handle.abort()` (the epoch manager pruning the instance below a frontier)
//! cascades to them; started from an outer context they would outlive it, since
//! `Handle` has no `Drop`.
//!
//! That external abort is a CANCELLATION, and what CANNOT be done under it is the
//! partition removal, which is async and therefore unreachable: the epoch manager
//! sweeps the leftovers by partition name instead
//! (`epoch_manager::prune_agreements`).

use commonware_consensus::{
    simplex::{self, config::ForwardingPolicy, elector::RoundRobin},
    types::{Epoch, ViewDelta},
};
use commonware_cryptography::Sha256;
use commonware_p2p::{Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_resolver::Resolver;
use commonware_runtime::{
    buffer::paged::CacheRef, BufferPooler, Clock, Handle, Metrics, Spawner, Storage,
};
use commonware_utils::ordered::BiMap;
use fluentbase_bls::{
    beacon::dkg_namespace, fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer,
    BlsPubkey, PeerPubkey, Scheme as BlsScheme,
};
use fluentbase_p2p::NoopBlocker;
use rand_core::CryptoRngCore;
use std::{collections::BTreeSet, time::Duration};
use tracing::{info, warn};

use crate::{
    beacon::{
        actor::{DkgLogIndex, PinnedMailbox, PinnedRequest},
        artifact::{ArtifactStore, CommitteeSource},
        dkg_agree::{
            note_omissions, AgreedArtifact, ConfirmPool, DkgAgree, DkgAgreeConfig, DkgReporter,
            PinnedLogs,
        },
        dkg_transport::{build_body_engine, register_dkg_subchannel},
        log_resolver::DkgLogKey,
        metrics::BeaconMetrics,
    },
    digest::Digest,
    outer::SharedMux,
    REPLAY_BUFFER, SCHEME_RETENTION_EPOCHS, WRITE_BUFFER,
};

/// Concurrent certificate-backfill requests, matching the ordering plane's.
const FETCH_CONCURRENT: usize = 4;

/// The six `simplex::Config` timeouts for the agreement instance.
///
/// Deliberately NOT [`crate::timeouts::ConsensusTimeouts`]: that struct's
/// `validated()` enforces two Fluent-only tripwires — `certification >= leader +
/// VERIFY_EXEC_BUDGET`, and `leader > BLOCK_INTERVAL` — that exist because a
/// consensus leader must EXECUTE a block inside its view. This instance executes
/// nothing, so it carries commonware's own invariants and no more.
#[derive(Clone, Copy, Debug)]
pub struct AgreementTimeouts {
    pub leader: Duration,
    pub certification: Duration,
    pub timeout_retry: Duration,
    pub fetch: Duration,
    pub activity: ViewDelta,
    pub skip: ViewDelta,
}

/// How long a view waits for its leader's proposal before nullifying — the value
/// that sets this instance's whole pace, because it is the ONLY timeout the happy
/// path never pays (the leader deadline clears the moment a proposal arrives,
/// builds or verifies, and one decision costs about three network delays).
///
/// It is coarse for TWO independent reasons, and both must survive any retune:
///
/// 1. **Journal file growth.** commonware's voter passes the raw view as the
///    journal SECTION, the manager holds one open blob per section with no cap
///    and no eviction, and the prune floor only ever moves on a FINALIZATION —
///    which a single-height instance reaches exactly once, at the end. There is
///    no seam to group sections or to prune from outside the voter, so the file
///    count is the view count and the only lever we hold is how slowly views
///    advance. A waiting view is measured at 218 B and one blob
///    (`a_waiting_view_is_bounded_by_open_blobs_and_not_by_bytes`), so it is the
///    blobs that bind and never the bytes: at 30 s a thousand of them take 8 h
///    20 min and cost 213 KiB, where at 750 ms they take 12 min 30 s. This is a
///    mitigation, not a fix — the fix is upstream.
/// 2. **The `interesting()` skew hazard.** A vote whose view is beyond
///    `current.next()` is DROPPED, and instances spawn on a local edge rather
///    than a shared one. With short views a skewed cohort drifts apart in view
///    number and silently discards each other's votes; with long views it stays
///    together in view 1 while it assembles.
const LEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// The rest of the view's budget after the leader deadline: proposal delivery,
/// `verify`, and vote collection. A view with an absent leader still costs only
/// [`LEADER_TIMEOUT`] — the leader deadline fires first and nullifies.
const CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(45);

/// Nullify re-broadcast cadence. Scaled with the coarse view, but NOT without
/// limit: the re-broadcast also re-ships the previous view's best certificate,
/// which is the laggard-repair path, so stretching it slows repair.
const TIMEOUT_RETRY: Duration = Duration::from_secs(5);

/// Certificate-backfill request timeout. This instance's payloads are kilobytes,
/// not a 4 MB block, so it does not inherit the ordering plane's budget.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

impl AgreementTimeouts {
    /// The coarse set. See [`LEADER_TIMEOUT`] for why coarse, and why the reason
    /// is two reasons.
    ///
    /// `activity` and `skip` govern a trailing window below the finalized tip
    /// that a single-height agreement does not have: `min_active` is
    /// `last_finalized - activity`, and this instance's `last_finalized` is the
    /// genesis view until the one finalization that ends it, so the floor is 0
    /// whatever they say. They are set to the ordering plane's values purely to
    /// satisfy commonware's construction asserts. `activity` is deliberately NOT
    /// raised above that: raising it only LOWERS the prune floor, so it can never
    /// improve retention and can only worsen it.
    pub const fn coarse() -> Self {
        Self {
            leader: LEADER_TIMEOUT,
            certification: CERTIFICATION_TIMEOUT,
            timeout_retry: TIMEOUT_RETRY,
            fetch: FETCH_TIMEOUT,
            activity: ViewDelta::new(64),
            skip: ViewDelta::new(4),
        }
    }

    /// Reproduce commonware's `Config::assert()` for the fields this struct owns,
    /// so a misconfiguration is an actionable error here instead of a panic deep
    /// inside `Engine::new`. Exactly that set — no Fluent tripwires.
    pub fn validated(self) -> Result<Self, &'static str> {
        if self.leader.is_zero()
            || self.certification.is_zero()
            || self.timeout_retry.is_zero()
            || self.fetch.is_zero()
        {
            return Err("all agreement timeouts must be greater than zero");
        }
        if self.activity.is_zero() || self.skip.is_zero() {
            return Err("activity_timeout / skip_timeout must be greater than zero");
        }
        if self.leader > self.certification {
            return Err(
                "leader_timeout > certification_timeout (commonware panics on construction)",
            );
        }
        if self.skip.get() > self.activity.get() {
            return Err("skip_timeout > activity_timeout");
        }
        Ok(self)
    }
}

/// Why an agreement instance could not be started. Every arm leaves the caller
/// free to retry on a later edge; none of them is fatal to the node.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AgreementError {
    /// This node's BLS key is not in `committee[target_epoch]`, so it is not a
    /// participant of the agreement it was asked to run. Not an error condition
    /// on the node — the caller simply does not run an instance.
    #[error("not a member of committee[{0}]")]
    NotAMember(u64),
    #[error("agreement timeouts are invalid: {0}")]
    Timeouts(&'static str),
}

/// Everything an agreement instance needs that is not a network channel.
pub(crate) struct AgreementConfig<P, R, L> {
    pub target_epoch: u64,
    pub chain_id: u64,
    /// `committee[target_epoch]` as the commonware-ordered `(peer, bls)` BiMap —
    /// the ONE index space the instance's signer slots, the dealer-log indices
    /// and the resolver's dealer identities are all numbered against.
    pub committee: BiMap<PeerPubkey, BlsPubkey>,
    pub keypair: ValidatorBlsKeypair,
    pub me: PeerPubkey,
    /// Peer provider for the body engine.
    pub peers: P,
    /// The existing `{epoch, dealer}` dealer-log resolver mailbox, which is what
    /// lets a parked `verify` drive the repair that ends its own park.
    pub logs: R,
    /// `epoch → idx → keccak256(SignedDealerLog)` for the logs this node holds.
    pub recorded: DkgLogIndex,
    /// The ceremony-state seam that turns a candidate pinned set into a group key
    /// (in production `crate::beacon::actor::PinnedMailbox`).
    pub pinned: L,
    /// The share-confirmations the entry bar counts, and the namespace they are
    /// signed under.
    ///
    /// PRECONDITION: the very pool the beacon actor was wired with. Its namespace is
    /// the one confirmations are verified under here; a second pool built from a
    /// different base would reject every honest confirmation and the bar would never
    /// be met, with no error anywhere — the plane would simply keep nullifying views.
    pub confirms: ConfirmPool,
    pub metrics: BeaconMetrics,
    /// Where the agreed artifact lands, and what peers are served from.
    ///
    /// Written HERE rather than by whoever reads `out`, so the artifact is
    /// servable the moment it exists even if nothing downstream is listening —
    /// and, once the store is opened against a partition, durable across the
    /// restart that today loses it outright (`dkg_agree_body_lost_total`).
    pub artifacts: ArtifactStore,
    pub mailbox_size: usize,
    pub timeouts: AgreementTimeouts,
    pub page_cache: CacheRef,
}

/// The four already-registered network routes an instance runs on: the three
/// `simplex` channels plus the body engine's.
pub(crate) struct AgreementNetworks<VS, VR, CS, CR, XS, XR, BS, BR> {
    pub vote: (VS, VR),
    pub cert: (CS, CR),
    pub resolver: (XS, XR),
    pub bodies: (BS, BR),
}

/// The journal partition for the agreement instance of `target_epoch`.
///
/// Disjoint from the ordering plane's `consensus_epoch_{n}` by name, and removed
/// wholesale after the abort — nothing else ever reclaims it.
pub fn agreement_partition(target_epoch: u64) -> String {
    format!("dkg_epoch_{target_epoch}")
}

/// Start the agreement instance for `cfg.target_epoch` and return the supervisor
/// handle that owns it.
///
/// The caller owns that handle and nothing else: aborting it tears down the
/// simplex engine and the body engine with it. The agreed artifact is delivered
/// on `out` AFTER the instance has been aborted and its partition destroyed, so a
/// consumer that acts on the artifact never races the teardown.
pub(crate) fn spawn_agreement<E, P, R, L, VS, VR, CS, CR, XS, XR, BS, BR>(
    context: E,
    cfg: AgreementConfig<P, R, L>,
    networks: AgreementNetworks<VS, VR, CS, CR, XS, XR, BS, BR>,
    out: tokio::sync::mpsc::Sender<AgreedArtifact>,
) -> Result<Handle<()>, AgreementError>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics,
    P: commonware_p2p::Provider<PublicKey = PeerPubkey>,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey>,
    L: PinnedLogs,
    VS: Sender<PublicKey = PeerPubkey>,
    VR: Receiver<PublicKey = PeerPubkey>,
    CS: Sender<PublicKey = PeerPubkey>,
    CR: Receiver<PublicKey = PeerPubkey>,
    XS: Sender<PublicKey = PeerPubkey>,
    XR: Receiver<PublicKey = PeerPubkey>,
    BS: Sender<PublicKey = PeerPubkey>,
    BR: Receiver<PublicKey = PeerPubkey>,
{
    let timeouts = cfg.timeouts.validated().map_err(AgreementError::Timeouts)?;
    let target_epoch = cfg.target_epoch;
    // SAFETY (§1): a base namespace distinct from AND prefix-free against the
    // chain's. The signed tuple carries only `Round{epoch, view}` and the
    // payload, with nothing identifying the instance, so under a shared base this
    // node's agreement vote and its ordering vote at the same `(epoch, view)` are
    // assemblable by ANY observer into equivocation evidence — and evidence
    // submission is permissionless.
    let namespace = dkg_namespace(&fluent_namespace(cfg.chain_id));
    // No `register_scheme`: see the module doc, point 2.
    let scheme = build_signer(&namespace, cfg.committee.clone(), &cfg.keypair, None)
        .ok_or(AgreementError::NotAMember(target_epoch))?;
    let committee: Vec<PeerPubkey> = cfg.committee.keys().iter().cloned().collect();
    let handle = context
        .with_label("dkg_agreement")
        .spawn(move |ctx| async move {
            let (bodies_engine, bodies) =
                build_body_engine(ctx.with_label("dkg_bodies"), cfg.me, cfg.peers);
            let bodies_handle = bodies_engine.start(networks.bodies);

            let agree = DkgAgree::new(
                ctx.with_label("dkg_agree"),
                DkgAgreeConfig {
                    target_epoch,
                    committee: committee.clone(),
                    bodies: bodies.clone(),
                    logs: cfg.logs,
                    recorded: cfg.recorded.clone(),
                    pinned: cfg.pinned,
                    confirms: cfg.confirms,
                    metrics: cfg.metrics.clone(),
                },
            );
            let (verdict_tx, mut verdict_rx) = tokio::sync::mpsc::channel(1);
            let reporter = DkgReporter::new(target_epoch, verdict_tx);

            let partition = agreement_partition(target_epoch);
            let engine = simplex::Engine::new(
                ctx.with_label("dkg_simplex"),
                simplex::Config {
                    scheme,
                    // Seedless by necessity: the VRF elector needs the very key this
                    // instance exists to agree.
                    elector: RoundRobin::<Sha256>::default(),
                    // The resolver's own excluded-set/quota defences are independent
                    // of this hook, and a real blocker here severs the peer from EVERY
                    // channel — a self-inflicted consensus partition paid for a
                    // benignly skewed peer on the agreement plane.
                    blocker: NoopBlocker,
                    automaton: agree.clone(),
                    relay: agree,
                    reporter,
                    strategy: Sequential,
                    partition: partition.clone(),
                    mailbox_size: cfg.mailbox_size,
                    epoch: Epoch::new(target_epoch),
                    replay_buffer: REPLAY_BUFFER,
                    write_buffer: WRITE_BUFFER,
                    page_cache: cfg.page_cache,
                    leader_timeout: timeouts.leader,
                    certification_timeout: timeouts.certification,
                    timeout_retry: timeouts.timeout_retry,
                    activity_timeout: timeouts.activity,
                    skip_timeout: timeouts.skip,
                    fetch_timeout: timeouts.fetch,
                    fetch_concurrent: FETCH_CONCURRENT,
                    forwarding: ForwardingPolicy::SilentLeader,
                },
            );
            let engine_handle = engine.start(networks.vote, networks.cert, networks.resolver);

            let certificate = verdict_rx.recv().await;

            // The instance cannot stop itself — `Engine::run` panics if any of its
            // actors returns — so the abort is ours, and it is unconditional: a
            // certificate means the agreement is decided, and a closed channel means
            // there is no longer anything to decide it.
            engine_handle.abort();
            // Joined before the partition is destroyed: `abort` only REQUESTS
            // cancellation — it cascades to the voter and the rest through the
            // runtime's supervision tree, but each of them stops at its own next
            // await point — and a journal write that landed after the remove would
            // leave a partition nothing ever reclaims.
            drop(engine_handle.await);

            let artifact = match certificate {
                Some(certificate) => {
                    let resolved = resolve_artifact(
                        &ctx,
                        &bodies,
                        &cfg.artifacts,
                        certificate,
                        timeouts.certification,
                    )
                    .await;
                    if resolved.is_none() {
                        cfg.metrics.dkg_agree_body_lost.inc();
                        warn!(
                            epoch = target_epoch,
                            "dkg agree: certified a payload whose body never arrived — the target \
                             epoch has to re-agree on a fresh instance"
                        );
                    }
                    resolved
                }
                None => {
                    warn!(
                        epoch = target_epoch,
                        "dkg agree: the instance ended without agreeing a set"
                    );
                    None
                }
            };

            bodies_handle.abort();
            drop(bodies_handle.await);
            if let Err(err) = ctx.remove(&partition, None).await {
                warn!(
                    epoch = target_epoch,
                    ?err,
                    "dkg agree: could not destroy the agreement journal partition"
                );
            }

            let Some(artifact) = artifact else {
                return;
            };
            // Before the send: a consumer of `out` that immediately serves or
            // re-publishes must never observe an artifact this node cannot yet
            // answer a peer's request for.
            cfg.artifacts.insert(target_epoch, artifact.clone());
            note_omissions(
                &cfg.recorded,
                target_epoch,
                committee.len(),
                &cfg.metrics,
                &artifact.0,
            );
            info!(
                epoch = target_epoch,
                view = artifact.1.proposal.round.view().get(),
                pinned = artifact.0.logs.len(),
                "dkg agree: pinned dealer-log set agreed, instance torn down"
            );
            let _ = out.send(artifact).await;
        });
    Ok(handle)
}

/// Pair a finalization certificate with the body it names.
///
/// Resolved HERE, with the body engine still up, and never in the reporter: the
/// reporter runs on the voter's own chain — journal replay awaits every `report`
/// inline — so all it could do there is an instant cache peek, and the peek is
/// wrong twice over. A certificate can outrun its body in normal operation (a node
/// that parked its whole `verify` on missing dealer logs learns the outcome from
/// the certificate alone), and after a restart the in-memory buffer is empty while
/// the certificate is replayed from the journal. `subscribe` is the await that the
/// first case satisfies.
///
/// The second case is what [`ArtifactStore`] now answers, and it is tried FIRST:
/// a restart rehydrates the store from its journal, so the body the empty
/// in-memory buffer cannot produce is already on disk and the wait is skipped
/// outright. The store is consulted only for a held artifact whose proposal
/// digests to the certificate's payload — first-wins per epoch makes a mismatch
/// unreachable, and treating it as an error here would be a verdict this function
/// has no committee read to justify, so it simply falls through to the wait.
///
/// What remains for the wait is the body that arrived NOWHERE — no store record
/// and no live sender, since nothing re-broadcasts a decided proposal. That is
/// why it still ends: waiting forever would leave the instance never torn down
/// and its journal partition never reclaimed, and the target epoch re-agrees on a
/// fresh instance instead. The bound is one view's certification budget because
/// that is exactly how long a live view gives a body to land.
async fn resolve_artifact<E: Clock>(
    ctx: &E,
    bodies: &crate::beacon::dkg_transport::BodyMailbox,
    artifacts: &ArtifactStore,
    certificate: commonware_consensus::simplex::types::Finalization<BlsScheme, Digest>,
    wait: Duration,
) -> Option<AgreedArtifact> {
    let epoch = certificate.proposal.round.epoch().get();
    if let Some(held) = artifacts.get(epoch) {
        if held.0.digest() == certificate.proposal.payload {
            return Some((held.0.clone(), certificate));
        }
    }
    let body = bodies.subscribe(certificate.proposal.payload).await;
    tokio::select! {
        received = body => received.ok().map(|proposal| (proposal, certificate)),
        _ = ctx.sleep(wait) => None,
    }
}

/// The four plane-owned mux brokers an agreement instance takes a sub-channel on.
///
/// The SAME brokers the per-epoch consensus engine registers against — the
/// instance takes a slice of the sub-channel id space no `register(epoch)` can
/// reach ([`dkg_subchannel`]), so no new top-level p2p channel, quota or peer set
/// is introduced for it.
pub struct AgreementMuxes<HS, HR>
where
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    pub vote: SharedMux<HS, HR>,
    pub cert: SharedMux<HS, HR>,
    pub resolver: SharedMux<HS, HR>,
    pub bodies: SharedMux<HS, HR>,
}

/// Everything an agreement instance needs that does NOT depend on which target
/// epoch it is for. The launcher holds one of these for the process and derives
/// an [`AgreementConfig`] per target from it.
pub struct AgreementPlaneConfig<P, R> {
    pub chain_id: u64,
    pub keypair: ValidatorBlsKeypair,
    pub me: PeerPubkey,
    /// PRECONDITION, and it is silent when violated: this must resolve
    /// `latest.primary` to a set containing `committee[target_epoch]`, or the body
    /// engine caches nothing and the plane never converges. In production it is
    /// the plane's own oracle, on which the `EpochTransition` tracks
    /// `active_registry ∪ committee[E]` — the same reachability the dealer-log
    /// resolver already rides.
    pub peers: P,
    /// The `{epoch, dealer}` dealer-log resolver, narrowed to the log key space.
    pub logs: R,
    pub recorded: DkgLogIndex,
    /// The beacon actor's pinned-set seam. One sender, one [`PinnedMailbox`] per
    /// instance — the epoch rides the request, so one actor serves every target.
    pub pinned_requests: tokio::sync::mpsc::Sender<PinnedRequest>,
    pub confirms: ConfirmPool,
    pub metrics: BeaconMetrics,
    pub artifacts: ArtifactStore,
    /// `committee[epoch]` as the on-chain staking read gives it — the same source
    /// the artifact seam verifies against, so an instance and a verifier can never
    /// disagree about who is in the committee.
    pub committee: CommitteeSource,
    pub mailbox_size: usize,
    pub timeouts: AgreementTimeouts,
}

/// Start the plane's launcher: one long-lived task that turns a target epoch on
/// `requests` into a running agreement instance.
///
/// The edge that feeds `requests` is the beacon actor's — a ceremony whose
/// dealing has CLOSED ([`crate::beacon::actor::DkgActor`]) — because that actor
/// owns the only state that knows when it happened. The launcher owns everything
/// the actor cannot reach: the mux registrations, the staking committee read and
/// the runtime context the instance is spawned on.
///
/// Requests are DEDUPLICATED here rather than at the sender, so a target whose
/// committee could not be read yet is simply retried on the next request for it
/// instead of being lost to a one-shot announcement.
///
/// Every started supervisor is handed to `adopted` — the epoch manager's intake —
/// which is what puts the instance under the same frontier cutoff as the per-epoch
/// engines. A send failure there is fatal to the instance's lifecycle management,
/// so it aborts the instance rather than leaking it.
#[allow(clippy::too_many_arguments)]
pub fn spawn_agreement_launcher<E, P, R, HS, HR>(
    context: E,
    cfg: AgreementPlaneConfig<P, R>,
    muxes: AgreementMuxes<HS, HR>,
    mut requests: tokio::sync::mpsc::Receiver<u64>,
    out: tokio::sync::mpsc::Sender<AgreedArtifact>,
    adopted: tokio::sync::mpsc::Sender<(Epoch, Handle<()>)>,
) -> Handle<()>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + Clone,
    P: commonware_p2p::Provider<PublicKey = PeerPubkey> + Clone + Sync,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey> + Clone + Sync,
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    context
        .clone()
        .with_label("dkg_agreement_launcher")
        .spawn(move |ctx| async move {
            // The instances' journal partitions are the only storage this plane
            // touches, and they are destroyed at teardown; a cache of its own
            // keeps that traffic off the ordering plane's.
            let page_cache = CacheRef::from_pooler(
                &ctx,
                crate::outer::PAGE_CACHE_PAGE_SIZE,
                crate::outer::PAGE_CACHE_CAPACITY,
            );
            let mut started: BTreeSet<u64> = BTreeSet::new();
            while let Some(target_epoch) = requests.recv().await {
                if started.contains(&target_epoch) {
                    continue;
                }
                let Some(committee) = (cfg.committee)(target_epoch) else {
                    warn!(
                        epoch = target_epoch,
                        "dkg agree: committee[epoch] is not readable yet; the instance will \
                         start on a later request"
                    );
                    continue;
                };
                match start_one(
                    &ctx,
                    &cfg,
                    &muxes,
                    page_cache.clone(),
                    target_epoch,
                    committee.bimap,
                    &out,
                )
                .await
                {
                    Started::Running(handle) => {
                        started.insert(target_epoch);
                        if adopted
                            .send((Epoch::new(target_epoch), handle))
                            .await
                            .is_err()
                        {
                            // Nothing owns the instance's lifetime any more: it
                            // would run past the frontier with no cutoff to prune
                            // it. The manager is gone, so the node is shutting
                            // down or demoted — either way, stop.
                            warn!(
                                epoch = target_epoch,
                                "dkg agree: nobody to adopt the instance; launcher exiting"
                            );
                            return;
                        }
                    }
                    // A settled answer for this target: never retry it.
                    Started::NotAMember => {
                        started.insert(target_epoch);
                    }
                    // Transient: the next request for this target retries.
                    Started::Failed => {}
                }
                // The request stream only ever moves forward, so anything a
                // retention window below the newest target will never be asked
                // for again.
                let floor = target_epoch.saturating_sub(SCHEME_RETENTION_EPOCHS as u64);
                started.retain(|e| *e >= floor);
            }
        })
}

/// What one launch attempt settled.
enum Started {
    Running(Handle<()>),
    /// This node is not in `committee[target_epoch]`, so there is no instance for
    /// it to run. Not an error and not retryable.
    NotAMember,
    /// A mux registration or a timeout check failed. Retryable.
    Failed,
}

/// Register the four sub-channels and start one instance.
async fn start_one<E, P, R, HS, HR>(
    ctx: &E,
    cfg: &AgreementPlaneConfig<P, R>,
    muxes: &AgreementMuxes<HS, HR>,
    page_cache: CacheRef,
    target_epoch: u64,
    committee: BiMap<PeerPubkey, BlsPubkey>,
    out: &tokio::sync::mpsc::Sender<AgreedArtifact>,
) -> Started
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + Clone,
    P: commonware_p2p::Provider<PublicKey = PeerPubkey> + Clone + Sync,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey> + Clone + Sync,
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    let mut routes = Vec::with_capacity(4);
    for mux in [&muxes.vote, &muxes.cert, &muxes.resolver, &muxes.bodies] {
        match register_dkg_subchannel(mux, target_epoch).await {
            Ok(route) => routes.push(route),
            Err(err) => {
                warn!(
                    epoch = target_epoch,
                    ?err,
                    "dkg agree: could not take the instance's sub-channel"
                );
                // The routes taken so far deregister as they drop.
                return Started::Failed;
            }
        }
    }
    let bodies = routes.pop().expect("four routes");
    let resolver = routes.pop().expect("four routes");
    let cert = routes.pop().expect("four routes");
    let vote = routes.pop().expect("four routes");

    match spawn_agreement(
        ctx.with_label(&format!("dkg_agree_e{target_epoch}")),
        AgreementConfig {
            target_epoch,
            chain_id: cfg.chain_id,
            committee,
            keypair: cfg.keypair.clone(),
            me: cfg.me.clone(),
            peers: cfg.peers.clone(),
            logs: cfg.logs.clone(),
            recorded: cfg.recorded.clone(),
            pinned: PinnedMailbox::new(target_epoch, cfg.pinned_requests.clone()),
            confirms: cfg.confirms.clone(),
            metrics: cfg.metrics.clone(),
            artifacts: cfg.artifacts.clone(),
            mailbox_size: cfg.mailbox_size,
            timeouts: cfg.timeouts,
            page_cache,
        },
        AgreementNetworks {
            vote,
            cert,
            resolver,
            bodies,
        },
        out.clone(),
    ) {
        Ok(handle) => {
            info!(
                epoch = target_epoch,
                "dkg agree: epoch-key agreement instance started"
            );
            Started::Running(handle)
        }
        Err(AgreementError::NotAMember(_)) => {
            info!(
                epoch = target_epoch,
                "dkg agree: not a member of committee[epoch]; no instance for this target"
            );
            Started::NotAMember
        }
        Err(err) => {
            warn!(
                epoch = target_epoch,
                ?err,
                "dkg agree: instance not started"
            );
            Started::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::artifact::ArtifactStore;
    use crate::beacon::{
        dkg_agree::{entry_bar, PinnedDerive, ShareConfirm},
        outcome::DkgOutcome,
    };
    use alloy_primitives::B256;
    use commonware_broadcast::Broadcaster as _;
    use commonware_codec::{varint::UInt, DecodeExt as _, EncodeSize as _};
    use commonware_consensus::{
        simplex::types::{Artifact, Nullification, Nullify},
        types::{Round, View},
    };
    use commonware_cryptography::{
        bls12381::{dkg::deal, primitives::sharing::Mode, primitives::variant::MinSig},
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network},
        Manager as _,
    };
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{
        ordered::Set, vec::NonEmptyVec, Faults as _, N3f1 as N3f1Faults, NZUsize, TryCollect as _,
    };
    use fluentbase_bls::{scheme::build_verifier, EpochCommittee};
    use fluentbase_p2p::constants::{self, MAX_COMMITTEE_SIZE};
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::{
        collections::BTreeMap,
        sync::{Arc, RwLock},
    };

    const TARGET: u64 = 6;
    const CHAIN_ID: u64 = 20_994;
    const N: usize = 4;

    /// In production each of these is a mux sub-channel; on the simulated network
    /// the four top-level routes stand in for them directly.
    const ROUTES: [(u64, commonware_runtime::Quota); 4] = [
        (constants::VOTE_CHANNEL, constants::VOTE_QUOTA),
        (constants::CERT_CHANNEL, constants::CERT_QUOTA),
        (constants::RESOLVER_CHANNEL, constants::RESOLVER_QUOTA),
        (constants::BROADCAST_CHANNEL, constants::BROADCAST_QUOTA),
    ];

    #[derive(Clone)]
    struct FixedPinned(Option<DkgOutcome>);

    impl PinnedLogs for FixedPinned {
        async fn derive(&self, _pinned: BTreeMap<u8, B256>) -> PinnedDerive {
            match &self.0 {
                Some(key) => PinnedDerive::Derived(Box::new(key.clone())),
                None => PinnedDerive::Unavailable,
            }
        }
    }

    #[derive(Clone, Default)]
    struct NoopResolver;

    impl Resolver for NoopResolver {
        type Key = DkgLogKey;
        type PublicKey = PeerPubkey;
        async fn fetch(&mut self, _: Self::Key) {}
        async fn fetch_all(&mut self, _: Vec<Self::Key>) {}
        async fn fetch_targeted(&mut self, _: Self::Key, _: NonEmptyVec<Self::PublicKey>) {}
        async fn fetch_all_targeted(&mut self, _: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>) {}
        async fn cancel(&mut self, _: Self::Key) {}
        async fn clear(&mut self) {}
        async fn retain(&mut self, _: impl Fn(&Self::Key) -> bool + Send + 'static) {}
    }

    fn group_key(seed: u64) -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..N).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        deal::<MinSig, PeerPubkey, N3f1Faults>(&mut rng, Mode::NonZeroCounter, players)
            .expect("deal")
            .0
    }

    /// Every seat's dealer log, so a proposer clears the quorum.
    fn full_logs() -> Vec<(u8, B256)> {
        (0..N as u8)
            .map(|i| (i, B256::repeat_byte(0x20 + i)))
            .collect()
    }

    fn full_index() -> DkgLogIndex {
        let entries: BTreeMap<u8, B256> = full_logs().into_iter().collect();
        Arc::new(RwLock::new(BTreeMap::from([(TARGET, entries)])))
    }

    #[test]
    fn timeouts_are_coarse_and_satisfy_the_construction_asserts() {
        let t = AgreementTimeouts::coarse().validated().expect("valid");
        assert!(t.leader <= t.certification);
        assert!(t.skip.get() <= t.activity.get());
        assert!(
            t.leader >= Duration::from_secs(30),
            "the leader timeout is what paces the instance; it must stay coarse"
        );
        let mut inverted = AgreementTimeouts::coarse();
        inverted.certification = inverted.leader - Duration::from_secs(1);
        assert!(inverted.validated().is_err());
        let mut skewed = AgreementTimeouts::coarse();
        skewed.skip = ViewDelta::new(skewed.activity.get() + 1);
        assert!(skewed.validated().is_err());
    }

    /// The two journal records a view that waits out its leader actually writes,
    /// in on-disk bytes: our own first-attempt `Nullify` vote and the recovered
    /// `Nullification` certificate. `certifiers` is how many of the `n` members
    /// signed into that certificate.
    ///
    /// The journal frames a record as an unsigned varint length followed by the
    /// encoding, with no checksum, and the voter builds its journal with
    /// compression disabled — so the on-disk record is exactly that sum.
    fn waiting_view_records(n: usize, certifiers: usize) -> (usize, usize) {
        let (peers, bls) = signing_set(0x5E, n);
        let bimap = bimap_of(&peers, &bls);
        let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
        let round = Round::new(Epoch::new(TARGET), View::new(1));
        let signers: Vec<BlsScheme> = bls
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
            .collect();
        let nullifies: Vec<Nullify<BlsScheme>> = signers
            .iter()
            .map(|s| Nullify::sign::<Digest>(s, round).expect("sign"))
            .collect();
        let nullification = Nullification::from_nullifies(
            &build_verifier(&ns, bimap, None, None),
            nullifies.iter().take(certifiers),
            &Sequential,
        )
        .expect("quorum");
        (
            record_bytes(&Artifact::Nullify(nullifies[0].clone())),
            record_bytes(&Artifact::Nullification(nullification)),
        )
    }

    fn record_bytes(artifact: &Artifact<BlsScheme, Digest>) -> usize {
        let item = artifact.encode_size();
        UInt(u32::try_from(item).expect("a record fits a u32 length")).encode_size() + item
    }

    /// Prices the trade the coarse [`LEADER_TIMEOUT`] makes. Without a finalization
    /// the prune floor never moves, so a spinning instance keeps every view's
    /// records AND one open blob per view, and the only lever we hold is how slowly
    /// views advance. Which of those two costs actually binds decides whether a
    /// slower clock is a sufficient answer or an evasion, so both are measured.
    ///
    /// Bytes do not bind, by three orders of magnitude: a waiting view writes about
    /// a fifth of a kilobyte, so the whole partition is still tens of kilobytes at a
    /// round count nobody expects a plane to reach. Open blobs are the real budget,
    /// and they are a pure function of the view count — which is what the coarse
    /// timeout buys down.
    ///
    /// The certificate carries a FIXED-WIDTH signer bitmap, so its size is set by
    /// the committee and not by how many of it signed: these are bounds on a
    /// waiting view, not samples of one.
    #[test]
    fn a_waiting_view_is_bounded_by_open_blobs_and_not_by_bytes() {
        const CAP: usize = MAX_COMMITTEE_SIZE as usize;
        let quorum = N3f1Faults::quorum(CAP) as usize;
        assert_eq!(
            waiting_view_records(CAP, quorum),
            waiting_view_records(CAP, CAP),
            "the signer bitmap is fixed-width; a fuller certificate must not cost more"
        );

        assert_eq!(
            waiting_view_records(N, N3f1Faults::quorum(N) as usize),
            (102, 110)
        );
        let (nullify, nullification) = waiting_view_records(CAP, quorum);
        assert_eq!((nullify, nullification), (102, 116));

        // Views on one target epoch beyond which the plane is spinning for a reason
        // nobody predicted — a yardstick for the byte budget, not a threshold the
        // code acts on.
        const SPINNING_ROUNDS: usize = 200;
        let at_the_yardstick = (nullify + nullification) * SPINNING_ROUNDS;
        assert!(
            at_the_yardstick < 64 * 1024,
            "a plane spinning past every plausible round count has written \
             {at_the_yardstick} B; if that ever approaches a real disk budget, bytes \
             have become a second constraint and the coarse timeout stops being a \
             sufficient answer"
        );
    }

    #[test]
    fn partition_is_disjoint_from_the_ordering_plane() {
        assert_eq!(agreement_partition(7), "dkg_epoch_7");
        assert_ne!(agreement_partition(7), format!("consensus_epoch_{}", 7));
    }

    fn signing_set(seed: u64, n: usize) -> (Vec<Ed25519PrivateKey>, Vec<ValidatorBlsKeypair>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let peers = (0..n)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls = (0..n)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        (peers, bls)
    }

    fn bimap_of(
        peers: &[Ed25519PrivateKey],
        bls: &[ValidatorBlsKeypair],
    ) -> BiMap<PeerPubkey, BlsPubkey> {
        peers
            .iter()
            .zip(bls.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                )
            })
            .try_collect()
            .expect("unique committee")
    }

    /// The two shapes a certificate can arrive in, and the reason the reporter no
    /// longer resolves the body itself.
    ///
    /// LATE BODY: the certificate outruns the body — a node that parked its whole
    /// `verify` on missing dealer logs learns the outcome from the certificate
    /// alone. An instant cache peek returns `None` here and the artifact is lost;
    /// the subscribe resolves the moment the body lands.
    ///
    /// NO BODY: the post-restart shape — the certificate is replayed from the
    /// journal and the in-memory buffer is empty. Nothing re-broadcasts a decided
    /// proposal, so the wait must END: a supervisor that waited forever would never
    /// tear the instance down and never reclaim its journal partition.
    #[test]
    fn a_certificate_is_paired_with_a_late_body_and_gives_up_when_there_is_none() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let mut rng = StdRng::seed_from_u64(41);
            let me = Ed25519PrivateKey::random(&mut rng).public_key();
            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            let channel = oracle
                .control(me.clone())
                .register(constants::BROADCAST_CHANNEL, constants::BROADCAST_QUOTA)
                .await
                .expect("register");
            oracle
                .manager()
                .track(0, Set::from_iter_dedup([me.clone()]))
                .await;
            let (engine, bodies) = crate::beacon::dkg_transport::build_body_engine(
                context.with_label("bodies"),
                me,
                oracle.manager(),
            );
            drop(engine.start(channel));

            let proposal = crate::beacon::dkg_agree::DkgProposal {
                target_epoch: TARGET,
                logs: (0..3u8).map(|i| (i, B256::repeat_byte(0x70 + i))).collect(),
                group_key: group_key(42),
                confirms: Vec::new(),
            };
            let digest = commonware_cryptography::Digestible::digest(&proposal);
            let certificate = finalization_over(digest);

            // NO BODY: the wait ends and the supervisor gets to tear down.
            assert!(
                resolve_artifact(
                    &context,
                    &bodies,
                    &ArtifactStore::new(),
                    certificate.clone(),
                    WAIT
                )
                .await
                .is_none(),
                "a certificate with no body must not park the teardown forever"
            );

            // LATE BODY: it lands after the subscribe, which an instant peek would
            // have missed.
            let late = {
                let bodies = bodies.clone();
                let context = context.clone();
                let proposal = proposal.clone();
                context
                    .clone()
                    .with_label("late")
                    .spawn(move |_| async move {
                        context.sleep(Duration::from_millis(200)).await;
                        drop(
                            bodies
                                .broadcast(commonware_p2p::Recipients::All, proposal)
                                .await,
                        );
                    })
            };
            let empty = ArtifactStore::new();
            let (artifact, _) = tokio::join!(
                resolve_artifact(&context, &bodies, &empty, certificate, WAIT),
                late
            );
            let (delivered, _cert) = artifact.expect("a late body must still be paired");
            assert_eq!(delivered, proposal);
        });
    }

    /// The post-restart shape, once the artifact store is durable: the certificate
    /// is replayed from the journal and the in-memory body buffer is empty, but the
    /// rehydrated artifact answers the pairing outright. The body engine here is
    /// built and NEVER started, so the only way to reach `subscribe` is to hang on
    /// it until the wait expires — which is what the clock assertion catches.
    #[test]
    fn a_rehydrated_artifact_short_circuits_the_wait() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let mut rng = StdRng::seed_from_u64(44);
            let me = Ed25519PrivateKey::random(&mut rng).public_key();
            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            let (_engine, bodies) = crate::beacon::dkg_transport::build_body_engine(
                context.with_label("bodies"),
                me,
                oracle.manager(),
            );

            let proposal = crate::beacon::dkg_agree::DkgProposal {
                target_epoch: TARGET,
                logs: (0..3u8).map(|i| (i, B256::repeat_byte(0x90 + i))).collect(),
                group_key: group_key(44),
                confirms: Vec::new(),
            };
            let certificate = finalization_over(proposal.digest());
            let store = ArtifactStore::new();
            assert!(store.insert(TARGET, (proposal.clone(), certificate.clone())));

            let before = context.current();
            let artifact = resolve_artifact(&context, &bodies, &store, certificate, WAIT).await;
            let (delivered, _cert) = artifact.expect("a held artifact must pair without the wait");
            assert_eq!(delivered, proposal);
            assert_eq!(
                context.current(),
                before,
                "the store answered, so not one tick of the certification budget may be spent"
            );
        });
    }

    /// Long enough to outlast the simulated network round-trips the late-body arm
    /// needs, and short enough that the no-body arm is a test and not a wait.
    const WAIT: Duration = Duration::from_secs(5);

    fn finalization_over(
        payload: Digest,
    ) -> commonware_consensus::simplex::types::Finalization<BlsScheme, Digest> {
        use commonware_consensus::simplex::types::{Finalize, Proposal};
        let (peers, bls) = signing_set(43, N);
        let bimap = bimap_of(&peers, &bls);
        let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
        let round = Round::new(Epoch::new(TARGET), View::new(1));
        let signers: Vec<BlsScheme> = bls
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
            .collect();
        let proposal = Proposal::new(round, View::new(0), payload);
        let finalizes: Vec<_> = signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, proposal.clone()).expect("sign"))
            .collect();
        commonware_consensus::simplex::types::Finalization::from_finalizes(
            &build_verifier(&ns, bimap, None, None),
            finalizes.iter(),
            &Sequential,
        )
        .expect("quorum")
    }

    /// A started four-member cohort on a simulated network, plus everything a test
    /// needs to steer it: the shared confirmation pool, the seat keys that can sign
    /// into it, and the group key every member derives.
    struct Cohort {
        handles: Vec<Handle<()>>,
        /// One artifact store per member, in the same seat order.
        stores: Vec<ArtifactStore>,
        out_rx: tokio::sync::mpsc::Receiver<AgreedArtifact>,
        pool: ConfirmPool,
        seat_keys: Vec<Ed25519PrivateKey>,
        members: Vec<PeerPubkey>,
        key: DkgOutcome,
        /// `committee[TARGET]` as a standalone verifier would read it off chain.
        committee: EpochCommittee,
    }

    impl Cohort {
        /// Sign `seats`' confirmations of the full dealer-log set into the shared
        /// pool — the statement a proposer needs to clear the entry bar. A range so
        /// a test can widen the confirmed set without re-recording what is already
        /// there, which the pool would (correctly) refuse as saying nothing new.
        fn confirm(&self, seats: std::ops::Range<usize>) {
            for (idx, key) in self
                .seat_keys
                .iter()
                .enumerate()
                .filter(|(idx, _)| seats.contains(idx))
            {
                let confirm =
                    ShareConfirm::sign(self.pool.namespace(), key, idx as u8, TARGET, full_logs());
                assert!(
                    self.pool.record(&self.members, confirm),
                    "the pool refused a genuine confirmation"
                );
            }
        }

        /// The artifact, or `None` if none arrives inside `window`.
        async fn artifact(
            &mut self,
            context: &deterministic::Context,
            window: Duration,
        ) -> Option<AgreedArtifact> {
            tokio::select! {
                _ = context.sleep(window) => None,
                artifact = self.out_rx.recv() => artifact,
            }
        }
    }

    /// Start one agreement instance per member over one simulated network. The
    /// confirmation pool is SHARED by all four, which models a cohort that has
    /// already gossiped: the confirmation transport itself is the beacon actor's and
    /// is tested there.
    async fn start_cohort(context: &deterministic::Context, seed: u64) -> Cohort {
        let (peers, bls) = signing_set(seed, N);
        let bimap = bimap_of(&peers, &bls);
        // The elector indexes the COMMONWARE-SORTED participant set, not the order
        // the keys were generated in, so the seats are read off the BiMap and every
        // per-seat decision below follows that order.
        let members: Vec<PeerPubkey> = bimap.keys().iter().cloned().collect();
        let keypairs: BTreeMap<PeerPubkey, ValidatorBlsKeypair> = peers
            .iter()
            .map(|p| p.public_key())
            .zip(bls.iter().cloned())
            .collect();
        let peer_keys: BTreeMap<PeerPubkey, Ed25519PrivateKey> =
            peers.iter().map(|p| (p.public_key(), p.clone())).collect();
        let seat_keys: Vec<Ed25519PrivateKey> =
            members.iter().map(|m| peer_keys[m].clone()).collect();
        let key = group_key(9);

        let (network, oracle) = Network::new(
            context.with_label("network"),
            SimConfig {
                max_size: 4 * 1024 * 1024,
                disconnect_on_block: false,
                tracked_peer_sets: NZUsize!(4),
            },
        );
        network.start();
        oracle
            .manager()
            .track(0, Set::from_iter_dedup(members.clone()))
            .await;

        let mut channels = Vec::new();
        for me in &members {
            let mut routes = Vec::new();
            for (channel, quota) in ROUTES {
                routes.push(
                    oracle
                        .control(me.clone())
                        .register(channel, quota)
                        .await
                        .expect("register"),
                );
            }
            channels.push(routes);
        }
        for a in 0..N {
            for b in 0..N {
                if a == b {
                    continue;
                }
                oracle
                    .add_link(
                        members[a].clone(),
                        members[b].clone(),
                        Link {
                            latency: Duration::from_millis(10),
                            jitter: Duration::from_millis(1),
                            success_rate: 1.0,
                        },
                    )
                    .await
                    .expect("link");
            }
        }

        let page_cache = CacheRef::from_pooler(
            context,
            crate::outer::PAGE_CACHE_PAGE_SIZE,
            crate::outer::PAGE_CACHE_CAPACITY,
        );
        let pool = ConfirmPool::new(b"FLUENT_TEST_COHORT");
        let stores: Vec<ArtifactStore> = (0..N).map(|_| ArtifactStore::new()).collect();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(N);
        let mut handles = Vec::new();
        for (i, mut nets) in channels.into_iter().enumerate() {
            let bodies = nets.pop().expect("body channel");
            let resolver = nets.pop().expect("resolver channel");
            let cert = nets.pop().expect("cert channel");
            let vote = nets.pop().expect("vote channel");
            // `RoundRobin` elects `(epoch + view) % n`, so at `TARGET = 6` and
            // `n = 4` view 1 elects seat 3. Give that seat nothing it can propose
            // and view 1 must nullify before any set is agreed.
            let silent = i == (TARGET as usize + 1) % N;
            let handle = spawn_agreement(
                context.with_label(&format!("member{i}")),
                AgreementConfig {
                    target_epoch: TARGET,
                    chain_id: CHAIN_ID,
                    committee: bimap.clone(),
                    keypair: keypairs[&members[i]].clone(),
                    me: members[i].clone(),
                    peers: oracle.manager(),
                    logs: NoopResolver,
                    recorded: if silent {
                        Arc::new(RwLock::new(BTreeMap::new()))
                    } else {
                        full_index()
                    },
                    pinned: FixedPinned(if silent { None } else { Some(key.clone()) }),
                    confirms: pool.clone(),
                    metrics: BeaconMetrics::default(),
                    artifacts: stores[i].clone(),
                    mailbox_size: 64,
                    // The coarse production set is asserted separately; here the
                    // leader timeout is only the thing view 1 has to run out, and
                    // the deterministic runtime pays for it in milliseconds of
                    // simulated cycles.
                    timeouts: AgreementTimeouts {
                        leader: Duration::from_secs(2),
                        certification: Duration::from_secs(3),
                        timeout_retry: Duration::from_millis(500),
                        fetch: Duration::from_millis(500),
                        ..AgreementTimeouts::coarse()
                    },
                    page_cache: page_cache.clone(),
                },
                AgreementNetworks {
                    vote,
                    cert,
                    resolver,
                    bodies,
                },
                out_tx.clone(),
            )
            .expect("member spawns an instance");
            handles.push(handle);
        }
        drop(out_tx);

        Cohort {
            handles,
            stores,
            out_rx,
            pool,
            seat_keys,
            members,
            key,
            committee: EpochCommittee::from_unverified(TARGET, bimap),
        }
    }

    /// The launcher: what turns the beacon actor's dealing-closed edge into a
    /// running instance, and the three behaviours that decide whether a target ever
    /// gets one.
    ///
    /// A request starts an instance and hands its supervisor to the epoch manager —
    /// without that hand-off nothing prunes it on the frontier cutoff. A repeat for
    /// a target already running is a no-op, which is what lets the actor re-announce
    /// on every height tick. And a request the plane could not act on — an
    /// unreadable `committee[epoch]`, which is the ordinary state until the executor
    /// reaches the block that committed it — must leave the target RETRYABLE: a
    /// one-shot announcement lost there would cost the epoch its instance outright.
    #[test]
    fn the_launcher_starts_one_instance_per_target_and_retries_an_unreadable_committee() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let (peers, bls) = signing_set(0x1A, N);
            let bimap = bimap_of(&peers, &bls);
            let members: Vec<PeerPubkey> = bimap.keys().iter().cloned().collect();
            let me = members[0].clone();
            let keypair = peers
                .iter()
                .zip(bls.iter())
                .find(|(p, _)| p.public_key() == me)
                .map(|(_, b)| b.clone())
                .expect("our keypair");

            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            oracle
                .manager()
                .track(0, Set::from_iter_dedup(members.clone()))
                .await;

            // One plane-owned broker per route, exactly as the node builds them.
            let mut muxes = Vec::new();
            for (i, (channel, quota)) in ROUTES.into_iter().enumerate() {
                let (sender, receiver) = oracle
                    .control(me.clone())
                    .register(channel, quota)
                    .await
                    .expect("register");
                let (muxer, handle) = commonware_p2p::utils::mux::Muxer::new(
                    context.with_label(&format!("mux{i}")),
                    sender,
                    receiver,
                    NZUsize!(16).get(),
                );
                muxer.start();
                muxes.push(std::sync::Arc::new(tokio::sync::Mutex::new(handle)));
            }

            // `committee[TARGET]` is readable; `committee[TARGET + 1]` is not, until
            // the flag flips — the executor-has-not-caught-up state.
            let readable = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let committee: CommitteeSource = {
                let bimap = bimap.clone();
                let readable = readable.clone();
                Arc::new(move |epoch: u64| {
                    if epoch != TARGET && !readable.load(std::sync::atomic::Ordering::Relaxed) {
                        return None;
                    }
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };

            let (request_tx, request_rx) = tokio::sync::mpsc::channel(8);
            let (out_tx, _out_rx) = tokio::sync::mpsc::channel(4);
            let (adopted_tx, mut adopted_rx) = tokio::sync::mpsc::channel(8);
            // Held, never read: `derive` parks, which is the honest local state for a
            // node whose ceremony cannot answer. The launcher is what is under test.
            let (pinned_tx, _pinned_rx) = tokio::sync::mpsc::channel(8);
            let launcher = spawn_agreement_launcher(
                context.with_label("launcher"),
                AgreementPlaneConfig {
                    chain_id: CHAIN_ID,
                    keypair,
                    me,
                    peers: oracle.manager(),
                    logs: NoopResolver,
                    recorded: full_index(),
                    pinned_requests: pinned_tx,
                    confirms: ConfirmPool::new(b"FLUENT_TEST_LAUNCHER"),
                    metrics: BeaconMetrics::default(),
                    artifacts: ArtifactStore::new(),
                    committee,
                    mailbox_size: 64,
                    timeouts: AgreementTimeouts {
                        leader: Duration::from_secs(2),
                        certification: Duration::from_secs(3),
                        timeout_retry: Duration::from_millis(500),
                        fetch: Duration::from_millis(500),
                        ..AgreementTimeouts::coarse()
                    },
                },
                AgreementMuxes {
                    vote: muxes[0].clone(),
                    cert: muxes[1].clone(),
                    resolver: muxes[2].clone(),
                    bodies: muxes[3].clone(),
                },
                request_rx,
                out_tx,
                adopted_tx,
            );

            request_tx.send(TARGET).await.expect("request");
            let (epoch, handle) = adopted_rx.recv().await.expect("an adopted supervisor");
            assert_eq!(epoch, Epoch::new(TARGET));

            // A repeat is a no-op: the actor re-announces on every height tick, and a
            // second instance for one target would agree against itself.
            request_tx.send(TARGET).await.expect("request");
            // An unreadable committee must not consume the target.
            request_tx.send(TARGET + 1).await.expect("request");
            assert!(
                tokio::select! {
                    _ = context.sleep(Duration::from_secs(2)) => true,
                    adopted = adopted_rx.recv() => {
                        assert!(adopted.is_none(), "a repeat or an unreadable committee started an instance");
                        false
                    }
                },
                "the launcher must have started nothing here"
            );

            readable.store(true, std::sync::atomic::Ordering::Relaxed);
            request_tx.send(TARGET + 1).await.expect("request");
            let (retried, retried_handle) = adopted_rx.recv().await.expect("the retry starts it");
            assert_eq!(
                retried,
                Epoch::new(TARGET + 1),
                "a target whose committee became readable must still get its instance"
            );

            handle.abort();
            retried_handle.abort();
            launcher.abort();
        });
    }

    /// The lifecycle end to end: view 1's round-robin leader has nothing it can
    /// propose, so the view nullifies; a later view finalizes; every supervisor
    /// returns on its own and the journal partitions are gone.
    #[test]
    fn a_nullified_first_view_still_finalizes_and_tears_down() {
        let runner = deterministic::Runner::timed(Duration::from_secs(3600));
        runner.start(|context| async move {
            let mut cohort = start_cohort(&context, 5).await;
            // At `n = 4` the entry bar is the bare quorum (`f = 1`, so the margin is
            // 0), and the silent seat confirms nothing.
            assert_eq!(entry_bar(N, View::new(1)), 3);
            cohort.confirm(0..3);

            let (proposal, finalization) = cohort.out_rx.recv().await.expect("an artifact");
            assert_eq!(proposal.target_epoch, TARGET);
            assert_eq!(proposal.group_key, cohort.key);
            assert_eq!(proposal.logs.len(), N);
            assert_eq!(
                proposal.confirms.len(),
                3,
                "the artifact carries the confirmations that cleared the bar"
            );
            assert!(
                finalization.proposal.round.view() > View::new(1),
                "view 1 was supposed to nullify, it finalized instead"
            );

            // Every supervisor returns on its own — the instance is aborted at the
            // finalization, not left running for the process lifetime.
            for handle in cohort.handles {
                handle.await.expect("supervisor returned cleanly");
            }

            // Every member kept the artifact it agreed, so a peer asking ANY of
            // them is served: the plane writes the store itself rather than
            // leaving it to whoever happens to read `out`. And each one verifies
            // against `committee[TARGET]` alone — no ceremony state, no share, no
            // block — which is the property the whole delivery path rests on.
            let mut rng = StdRng::seed_from_u64(2026);
            for (i, store) in cohort.stores.iter().enumerate() {
                let held = store
                    .get(TARGET)
                    .unwrap_or_else(|| panic!("member {i} agreed and kept nothing"));
                assert_eq!(held.0, proposal, "member {i} stored a different value");
                crate::beacon::artifact::verify_artifact(
                    &mut rng,
                    CHAIN_ID,
                    &cohort.committee,
                    &held,
                )
                .expect("a live artifact must verify against the committee alone");
            }

            for epoch in [TARGET] {
                assert!(
                    context
                        .scan(&agreement_partition(epoch))
                        .await
                        .map(|blobs| blobs.is_empty())
                        .unwrap_or(true),
                    "the agreement journal partition survived the teardown"
                );
            }
        });
    }

    /// Below the entry bar the plane NEVER aborts. It nullifies view after view and
    /// keeps every instance alive, so the epoch is still agreed the moment enough
    /// members confirm — minutes later, with no operator action and no restart.
    ///
    /// The second half is what makes the first half an assertion rather than a
    /// timeout: a plane that had given up would produce no artifact after the
    /// confirmations land either.
    #[test]
    fn the_plane_waits_below_the_entry_bar_and_never_aborts() {
        let runner = deterministic::Runner::timed(Duration::from_secs(3600));
        runner.start(|context| async move {
            let mut cohort = start_cohort(&context, 5).await;
            // One short of the bar: every honest leader refuses to propose, and no
            // verifier would accept it if one did.
            let bar = entry_bar(N, View::new(1));
            cohort.confirm(0..bar - 1);
            // Ten leader timeouts: every seat leads at least twice and refuses every
            // time. Longer buys no more confidence and every view costs real BLS.
            assert!(
                cohort
                    .artifact(&context, Duration::from_secs(20))
                    .await
                    .is_none(),
                "the plane agreed a value below the entry bar"
            );

            cohort.confirm(bar - 1..bar);
            let (proposal, _) = cohort
                .artifact(&context, Duration::from_secs(120))
                .await
                .expect("the plane must still be running, and must agree once confirmed");
            assert_eq!(proposal.logs.len(), N);

            for handle in cohort.handles {
                handle.await.expect("supervisor returned cleanly");
            }
        });
    }
}

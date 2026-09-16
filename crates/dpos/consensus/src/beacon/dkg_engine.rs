//! The epoch-key agreement instance: a second, short-lived `simplex` over
//! `committee[target_epoch]`, and the supervisor that ends it.
//!
//! It runs on its own namespace ([`fluentbase_bls::beacon::dkg_namespace`]) and
//! its own sub-channels, agrees exactly one value — the pinned dealer-log set —
//! and is torn down at its own finalization. It differs from the ordering plane's
//! engine in five load-bearing ways:
//!
//! 1. a distinct, prefix-free namespace, or an observer could assemble an honest
//!    validator's two payloads for one `(epoch, view)` into equivocation evidence;
//! 2. no `register_scheme`, because the shared provider refuses a
//!    different-committee re-register and writing this instance's scheme there
//!    would drop the ordering engine's;
//! 3. a seedless [`RoundRobin`] elector, since the VRF elector needs the very key
//!    this instance exists to agree;
//! 4. a reporter wired to neither marshal, slasher nor `spec_exec`, because
//!    agreement rounds are not consensus rounds;
//! 5. its own journal partition, destroyed after the abort.
//!
//! `simplex::Engine::run` panics if any of its actors finishes, so the abort comes
//! from outside: [`spawn_agreement`] returns one handle and the supervisor aborts
//! the instance when its reporter delivers a finalization. The engines start from
//! the supervisor's own context, so an external abort cascades to them.
//!
//! The launcher ([`spawn_agreement_launcher`]) is the one owner of every started
//! instance and of its journal partition: it starts an instance on the actor's
//! dealing-closed edge, holds its handle, and aborts and joins it when the epoch
//! clock moves past the target, sweeping the partition band below the cutoff.

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
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
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
        AGREEMENT_JOURNAL_PARTITION_PREFIX,
    },
    digest::Digest,
    outer::SharedMux,
    sync_metrics::SafetyHalt,
    REPLAY_BUFFER, SCHEME_RETENTION_EPOCHS, WRITE_BUFFER,
};

/// Concurrent certificate-backfill requests, matching the ordering plane's.
const FETCH_CONCURRENT: usize = 4;

/// The six `simplex::Config` timeouts for the agreement instance.
///
/// Not [`crate::timeouts::ConsensusTimeouts`]: that struct enforces two
/// consensus-specific tripwires for a leader that must execute a block inside its
/// view. This instance executes nothing, so it carries commonware's own invariants
/// and no more.
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
/// that sets the instance's pace, because it is the only timeout the happy path
/// never pays.
///
/// It is coarse for two independent reasons. commonware's voter uses the raw view as
/// the journal section and the prune floor only moves on a finalization, which a
/// single-height instance reaches once, so the file count is the view count and the
/// only lever is how slowly views advance. And a vote whose view is beyond
/// `current.next()` is dropped while instances spawn on a local edge, so short views
/// let a skewed cohort drift apart in view number.
const LEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// The rest of the view's budget after the leader deadline: proposal delivery,
/// `verify`, and vote collection. A view with an absent leader still costs only
/// [`LEADER_TIMEOUT`] — the leader deadline fires first and nullifies.
const CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(45);

/// Nullify re-broadcast cadence, scaled with the coarse view. The re-broadcast also
/// re-ships the previous view's best certificate, so stretching it slows repair.
const TIMEOUT_RETRY: Duration = Duration::from_secs(5);

/// Certificate-backfill request timeout. This instance's payloads are kilobytes,
/// not a 4 MB block, so it does not inherit the ordering plane's budget.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// The coarse set. `activity` and `skip` govern a trailing window below the
/// finalized tip that a single-height agreement does not have, so the floor is 0
/// whatever they say; they are set to the ordering plane's values only to satisfy
/// commonware's construction asserts.
impl AgreementTimeouts {
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
    /// The existing `{epoch, dealer, hash}` dealer-log resolver mailbox, which is what
    /// lets a parked `verify` drive the repair that ends its own park.
    pub logs: R,
    /// `epoch → idx → keccak256(SignedDealerLog)` for the logs this node holds.
    pub recorded: DkgLogIndex,
    /// The ceremony-state seam that turns a candidate pinned set into a group key
    /// (in production `crate::beacon::actor::PinnedMailbox`).
    pub pinned: L,
    /// Precondition: the very pool the beacon actor was wired with. A second pool
    /// built from a different base would reject every honest confirmation and the bar
    /// would never be met, with no error anywhere.
    pub confirms: ConfirmPool,
    pub metrics: BeaconMetrics,
    /// Where the agreed artifact lands, and what peers are served from. Written here so
    /// the artifact is servable the moment it exists and durable across a restart.
    pub artifacts: ArtifactStore,
    pub mailbox_size: usize,
    pub timeouts: AgreementTimeouts,
    pub page_cache: CacheRef,
    /// See `agreement_partition`.
    pub partition_prefix: String,
    /// Where a certified-but-unresolved body is reported (the target epoch): the
    /// DKG actor moves that epoch to acquiring the artifact from peers at once.
    /// `None` means nobody to tell (the instance's unit tests).
    pub body_lost: Option<tokio::sync::mpsc::Sender<u64>>,
}

/// The four already-registered network routes an instance runs on: the three
/// `simplex` channels plus the body engine's.
pub(crate) struct AgreementNetworks<VS, VR, CS, CR, XS, XR, BS, BR> {
    pub vote: (VS, VR),
    pub cert: (CS, CR),
    pub resolver: (XS, XR),
    pub bodies: (BS, BR),
}

/// The journal partition for the agreement instance of `target_epoch`, under
/// `prefix` (production passes `""`). Disjoint from the ordering plane's
/// `consensus_epoch_{n}` by name, and removed wholesale after the abort.
fn agreement_partition(prefix: &str, target_epoch: u64) -> String {
    format!("{prefix}{AGREEMENT_JOURNAL_PARTITION_PREFIX}{target_epoch}")
}

/// Target epochs below the cutoff whose agreement journal partition is swept on
/// every prune.
///
/// A supervisor removes its own partition after it delivers, but an external abort
/// cancels it at an await and the removal never runs, so the band below the cutoff
/// is swept by epoch number — which also collects a previous process's leftovers.
/// The sweep follows the two edges that can put a partition in the band: the cutoff
/// reaching a new epoch, or this call having aborted an instance.
const AGREEMENT_SWEEP_SPAN: u64 = SCHEME_RETENTION_EPOCHS as u64;

/// Drop every agreement instance whose target is below `cutoff`, aborting it on the
/// way out, and reclaim the journal partitions below the cutoff.
///
/// `swept_to` is the highest cutoff whose band this process has already swept. It is
/// raised rather than assigned, so a prune below the highest cutoff that aborted
/// something sweeps its own band without un-sweeping the higher one.
async fn prune_agreements<E: Storage>(
    context: &E,
    agreements: &mut BTreeMap<Epoch, Handle<()>>,
    cutoff: u64,
    partition_prefix: &str,
    swept_to: &mut u64,
) {
    let stale: Vec<Epoch> = agreements
        .keys()
        .copied()
        .filter(|e| e.get() < cutoff)
        .collect();
    let aborted_one = !stale.is_empty();
    for e in stale {
        if let Some(handle) = agreements.remove(&e) {
            handle.abort();
            drop(handle.await);
            info!(?e, "epoch-key agreement instance pruned (transition)");
        }
    }
    if !(cutoff > *swept_to || aborted_one) {
        return;
    }
    for epoch in cutoff.saturating_sub(AGREEMENT_SWEEP_SPAN)..cutoff {
        // A live instance still owns its partition even below the cutoff: it was
        // started after this prune's abort pass.
        if agreements.contains_key(&Epoch::new(epoch)) {
            continue;
        }
        match context
            .remove(&agreement_partition(partition_prefix, epoch), None)
            .await
        {
            Ok(()) => info!(epoch, "epoch-key agreement journal partition reclaimed"),
            // Nothing to reclaim: the overwhelmingly common case, since the band
            // is swept whenever it moves whether or not an instance ever ran
            // there.
            Err(commonware_runtime::Error::PartitionMissing(_)) => {}
            Err(err) => warn!(
                epoch,
                ?err,
                "could not reclaim the epoch-key agreement journal partition"
            ),
        }
    }
    *swept_to = (*swept_to).max(cutoff);
}

/// Start the agreement instance for `cfg.target_epoch` and return the supervisor
/// handle that owns it.
///
/// Aborting the handle tears down the simplex engine and the body engine with it.
/// The artifact is delivered on `out` after the instance has been aborted and its
/// partition destroyed, so a consumer never races the teardown.
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
    // assemblable by any observer into equivocation evidence — and evidence
    // submission is permissionless.
    let namespace = dkg_namespace(&fluent_namespace(cfg.chain_id));
    // No `register_scheme`: see the module doc, point 2.
    // Seedless by construction: the instance that agrees the epoch key cannot
    // depend on it, so it never carries an oracle.
    let scheme = build_signer(
        &namespace,
        cfg.committee.clone(),
        &cfg.keypair,
        target_epoch,
        None,
    )
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

            let partition = agreement_partition(&cfg.partition_prefix, target_epoch);
            let engine = simplex::Engine::new(
                ctx.with_label("dkg_simplex"),
                simplex::Config {
                    scheme,
                    // Seedless by necessity: the VRF elector needs the very key this
                    // instance exists to agree.
                    elector: RoundRobin::<Sha256>::default(),
                    // The resolver's own defences are independent of this hook, and a real blocker
                    // here would sever the peer from every channel over a benign skew.
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
            // Joined before the partition is destroyed: `abort` only requests
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
                            "dkg agree: certified a payload whose body never arrived — the \
                             artifact is acquired from a peer that resolved it"
                        );
                        // Tell the actor now: no instance for this target runs again in this
                        // process, so the only source of the artifact is a peer's copy
                        // and the actor owns the pull.
                        if let Some(tx) = &cfg.body_lost {
                            let _ = tx.try_send(target_epoch);
                        }
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
            // Before the send: a consumer must never observe an artifact this node cannot yet
            // serve. A store that already holds another value for this target keeps it
            // first-wins and notes the conflict for the actor's next tick.
            if let Err(loser) = cfg.artifacts.insert(target_epoch, artifact.clone()) {
                cfg.artifacts.note_divergent(target_epoch, &loser);
            }
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
/// Resolved here, with the body engine still up, and never in the reporter: the
/// reporter runs on the voter's own chain, so all it could do is an instant cache
/// peek, which a certificate that outruns its body or a post-restart empty buffer
/// would defeat. The artifact store is tried first, then the live body.
///
/// A body that arrived nowhere still ends the wait after one view's certification
/// budget, so the instance is torn down and its partition reclaimed.
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
/// The same brokers the per-epoch consensus engine registers against, so no new
/// top-level p2p channel, quota or peer set is introduced.
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

/// Everything an agreement instance needs that does not depend on which target
/// epoch it is for. The launcher holds one of these for the process and derives
/// an [`AgreementConfig`] per target from it.
pub struct AgreementPlaneConfig<P, R> {
    pub chain_id: u64,
    pub keypair: ValidatorBlsKeypair,
    pub me: PeerPubkey,
    /// Precondition, silent when violated: this must resolve `latest.primary` to a set
    /// containing `committee[target_epoch]`, or the body engine caches nothing and the
    /// plane never converges. In production it is the plane's own oracle, which tracks
    /// `committee[E-1] ∪ committee[E] ∪ committee[E+1]` as primary.
    pub peers: P,
    /// The `{epoch, dealer, hash}` dealer-log resolver, narrowed to the log key space.
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
    /// See `agreement_partition`. Production passes `""`.
    pub partition_prefix: String,
    /// See [`AgreementConfig::body_lost`].
    pub body_lost: Option<tokio::sync::mpsc::Sender<u64>>,
    /// The launcher task's state after every event it handled, for the tests
    /// that drive the real task ([`spawn_agreement_launcher`]) and have no other
    /// view into it. `None` in production.
    #[cfg(test)]
    pub probe: Option<watch::Sender<LauncherProbe>>,
}

/// What [`AgreementPlaneConfig::probe`] publishes: the number of events the
/// task has handled (a request, a clock edge, the halt edge) and the targets it
/// holds an instance for.
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LauncherProbe {
    pub events: u64,
    pub instances: Vec<u64>,
}

/// Start the plane's launcher: one long-lived task that turns a target epoch on
/// `requests` into a running agreement instance.
///
/// The edge that feeds `requests` is the beacon actor's dealing-closed edge. The
/// launcher owns the mux registrations, the staking committee read and the runtime
/// context the instance is spawned on. Requests are deduplicated here, so a target
/// whose committee could not be read yet is retried rather than lost.
///
/// `clock` is the actor's epoch clock, the cutoff `prune_agreements` runs on, and
/// `safety_halt`'s 0→1 edge aborts every running instance at once.
#[allow(clippy::too_many_arguments)]
pub fn spawn_agreement_launcher<E, P, R, HS, HR>(
    context: E,
    cfg: AgreementPlaneConfig<P, R>,
    muxes: AgreementMuxes<HS, HR>,
    mut requests: mpsc::Receiver<u64>,
    out: mpsc::Sender<AgreedArtifact>,
    mut clock: watch::Receiver<u64>,
    safety_halt: SafetyHalt,
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
            #[cfg(test)]
            let probe = cfg.probe.clone();
            #[cfg(test)]
            let mut events = 0u64;
            let mut launcher = Launcher::new(ctx, cfg, muxes, out, safety_halt.clone());
            // The latch's 0→1 edge, armed once: an already-engaged latch resolves it on the
            // first select, and re-arming it would spin the loop.
            let halt_edge = safety_halt.engaged_edge();
            tokio::pin!(halt_edge);
            let mut halt_seen = false;
            loop {
                tokio::select! {
                    // The halt arm is polled first: an engaged latch wins every iteration it
                    // is ready on, so an instance never outlives it by a request or a tick.
                    biased;
                    _ = &mut halt_edge, if !halt_seen => {
                        halt_seen = true;
                        launcher.on_halt().await;
                    }
                    request = requests.recv() => match request {
                        Some(target_epoch) => launcher.on_request(target_epoch).await,
                        // The actor is gone: nothing will ask for an instance again.
                        None => break,
                    },
                    changed = clock.changed() => match changed {
                        Ok(()) => {
                            let cutoff = *clock.borrow_and_update();
                            launcher.on_tick(cutoff).await;
                        }
                        // The clock's writer is the actor: with it gone nothing
                        // would ever prune what this task holds.
                        Err(_) => break,
                    },
                }
                #[cfg(test)]
                if let Some(probe) = &probe {
                    events += 1;
                    probe.send_replace(LauncherProbe {
                        events,
                        instances: launcher.instances.keys().map(|e| e.get()).collect(),
                    });
                }
            }
            launcher.abort_all("aborting epoch-key agreement instance on exit");
        })
}

/// The launcher's state: everything an instance is started from, and the map of
/// instances started. One task owns it, so an instance is in the map from the
/// moment it is spawned.
struct Launcher<E, P, R, HS, HR>
where
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    ctx: E,
    cfg: AgreementPlaneConfig<P, R>,
    muxes: AgreementMuxes<HS, HR>,
    /// The instances' journal partitions are the only storage this plane touches, and
    /// they are destroyed at teardown.
    ///
    /// The node's fork-safety latch: a halted node runs no agreement instance, so it is
    /// read at every spawn and re-read after one, and its 0→1 edge aborts the instances
    /// live at that moment.
    page_cache: CacheRef,
    out: mpsc::Sender<AgreedArtifact>,
    safety_halt: SafetyHalt,
    /// Supervisor handles of the running instances, keyed by target epoch. An instance
    /// aborts itself when its finalization lands, so nothing polls a handle for death.
    instances: BTreeMap<Epoch, Handle<()>>,
    /// Targets settled in this process — running, not a member, or refused by
    /// the latch — so the actor's per-tick re-announcement is a no-op for them.
    started: BTreeSet<u64>,
    /// The highest cutoff whose partition band [`prune_agreements`] has already swept
    /// in this process; `0` makes a fresh process's first tick sweep.
    swept_to: u64,
}

impl<E, P, R, HS, HR> Launcher<E, P, R, HS, HR>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + Clone,
    P: commonware_p2p::Provider<PublicKey = PeerPubkey> + Clone + Sync,
    R: Resolver<Key = DkgLogKey, PublicKey = PeerPubkey> + Clone + Sync,
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    fn new(
        ctx: E,
        cfg: AgreementPlaneConfig<P, R>,
        muxes: AgreementMuxes<HS, HR>,
        out: mpsc::Sender<AgreedArtifact>,
        safety_halt: SafetyHalt,
    ) -> Self {
        let page_cache = CacheRef::from_pooler(
            &ctx,
            crate::outer::PAGE_CACHE_PAGE_SIZE,
            crate::outer::PAGE_CACHE_CAPACITY,
        );
        Self {
            ctx,
            cfg,
            muxes,
            page_cache,
            out,
            safety_halt,
            instances: BTreeMap::new(),
            started: BTreeSet::new(),
            swept_to: 0,
        }
    }

    /// The actor's dealing-closed edge for `target_epoch`: start its instance,
    /// unless this target is settled, its committee is not readable yet, or the
    /// node is halted.
    async fn on_request(&mut self, target_epoch: u64) {
        if self.started.contains(&target_epoch) {
            return;
        }
        self.start(target_epoch).await;
        // The request stream only ever moves forward, so anything a retention
        // window below the newest target will never be asked for again. On every
        // outcome — a refused or an unreadable target is a request too, and a
        // halted node's `started` would otherwise grow by one per announced epoch
        // for the rest of the process.
        let floor = target_epoch.saturating_sub(SCHEME_RETENTION_EPOCHS as u64);
        self.started.retain(|e| *e >= floor);
    }

    /// The body of [`Self::on_request`] after the dedup: every early return is
    /// an outcome the caller's bookkeeping still runs for.
    async fn start(&mut self, target_epoch: u64) {
        // The latch, before the spawn. Permanent, so the target is settled like a
        // non-member's: the warn is once per target, not once per tick.
        if self.safety_halt.is_engaged() {
            warn!(
                epoch = target_epoch,
                "SafetyHalt engaged — not starting an epoch-key agreement instance"
            );
            self.started.insert(target_epoch);
            return;
        }
        let Some(committee) = (self.cfg.committee)(target_epoch) else {
            warn!(
                epoch = target_epoch,
                "dkg agree: committee[epoch] is not readable yet; the instance will start on \
                 a later request"
            );
            return;
        };
        match start_one(
            &self.ctx,
            &self.cfg,
            &self.muxes,
            self.page_cache.clone(),
            target_epoch,
            committee.bimap,
            &self.out,
        )
        .await
        {
            Started::Running(handle) => {
                self.started.insert(target_epoch);
                // Unreachable while `started` dedups, kept because the map is the
                // owner: two instances must never run for one target.
                if let Some(previous) = self.instances.insert(Epoch::new(target_epoch), handle) {
                    warn!(
                        epoch = target_epoch,
                        "replacing a live epoch-key agreement instance"
                    );
                    previous.abort();
                }
            }
            // A settled answer for this target: never retry it.
            Started::NotAMember => {
                self.started.insert(target_epoch);
            }
            // Transient: the next request for this target retries.
            Started::Failed => {}
        }
        // The latch after the spawn as well, on every outcome: `start_one` awaited
        // the four mux registrations, and a latch that engaged during them has
        // its edge queued behind this request on the launcher's `select!` — the
        // instance just started, or one already in the map, would run until the
        // next iteration polled that arm. Retire them here instead, on the same
        // task, before anything else runs.
        if self.safety_halt.is_engaged() {
            self.on_halt().await;
        }
    }

    /// An edge of the actor's epoch clock, `cutoff` being the epoch it entered:
    /// prune below it.
    async fn on_tick(&mut self, cutoff: u64) {
        prune_agreements(
            &self.ctx,
            &mut self.instances,
            cutoff,
            &self.cfg.partition_prefix,
            &mut self.swept_to,
        )
        .await;
    }

    /// The fork-safety latch's edge: abort and join every running instance, whatever
    /// its target. Joined like the prune's aborts, so a later sweep cannot race a last
    /// journal write. Idempotent.
    async fn on_halt(&mut self) {
        for (epoch, handle) in std::mem::take(&mut self.instances) {
            warn!(
                ?epoch,
                "SafetyHalt engaged — aborting epoch-key agreement instance"
            );
            handle.abort();
            drop(handle.await);
        }
    }

    /// Abort every instance. Supervision would do it for the task's exit as well; this
    /// is the log line.
    fn abort_all(&mut self, why: &'static str) {
        for (epoch, handle) in std::mem::take(&mut self.instances) {
            info!(?epoch, "{why}");
            handle.abort();
        }
    }
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
            partition_prefix: cfg.partition_prefix.clone(),
            body_lost: cfg.body_lost.clone(),
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

    /// In production each of these is a mux sub-channel; on the simulated network the
    /// four top-level routes stand in directly.
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

    /// The two journal records a view that waits out its leader writes, in on-disk
    /// bytes: the `Nullify` vote and the `Nullification` certificate.
    fn waiting_view_records(n: usize, certifiers: usize) -> (usize, usize) {
        let (peers, bls) = signing_set(0x5E, n);
        let bimap = bimap_of(&peers, &bls);
        let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
        let round = Round::new(Epoch::new(TARGET), View::new(1));
        let signers: Vec<BlsScheme> = bls
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, TARGET, None).expect("member"))
            .collect();
        let nullifies: Vec<Nullify<BlsScheme>> = signers
            .iter()
            .map(|s| Nullify::sign::<Digest>(s, round).expect("sign"))
            .collect();
        let nullification = Nullification::from_nullifies(
            &build_verifier(&ns, bimap, TARGET, None),
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

    /// Prices the trade the coarse [`LEADER_TIMEOUT`] makes. Without a finalization the
    /// prune floor never moves, so a spinning instance keeps every view's records and
    /// one open blob per view, and the only lever is how slowly views advance.
    ///
    /// The certificate's signer bitmap is fixed-width, so its size is set by the
    /// committee and not by how many of it signed.
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
        // An empty prefix is the production spelling: the on-disk names must not
        // move when the prefix parameter is threaded through.
        assert_eq!(agreement_partition("", 7), "dkg_epoch_7");
        assert_eq!(crate::engine::engine_partition("", 7), "consensus_epoch_7");
        assert_ne!(
            agreement_partition("", 7),
            crate::engine::engine_partition("", 7)
        );
        // A per-node prefix keeps the two planes disjoint and separates nodes.
        assert_eq!(agreement_partition("node2-", 7), "node2-dkg_epoch_7");
        assert_eq!(
            crate::engine::engine_partition("node2-", 7),
            "node2-consensus_epoch_7"
        );
        assert_ne!(
            crate::engine::engine_partition("node1-", 7),
            crate::engine::engine_partition("node2-", 7)
        );
    }

    /// The agreement instances live in the launcher's map and are pruned on the actor's
    /// epoch clock; pruning aborts and joins them, so a still-stopping instance cannot
    /// have its partition removed and then recreated by a last journal write.
    #[test]
    fn agreement_instances_prune_on_the_clock_cutoff_and_are_aborted() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Set on drop, which for an aborted task is the moment its future is dropped —
        /// the observable proof that `abort()` reached it.
        struct Tombstone(Arc<AtomicUsize>);
        impl Drop for Tombstone {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            let dropped = Arc::new(AtomicUsize::new(0));
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            for epoch in [3u64, 4, 5] {
                let mark = Tombstone(dropped.clone());
                agreements.insert(
                    Epoch::new(epoch),
                    ctx.with_label("agreement").spawn(move |_| async move {
                        let _mark = mark;
                        std::future::pending::<()>().await;
                    }),
                );
            }
            ctx.sleep(Duration::from_millis(1)).await;

            prune_agreements(&ctx, &mut agreements, 5, "", &mut 0).await;
            assert_eq!(
                agreements.keys().copied().collect::<Vec<_>>(),
                vec![Epoch::new(5)],
                "the cutoff epoch's own instance must survive its cutoff"
            );

            assert_eq!(
                dropped.load(Ordering::SeqCst),
                2,
                "prune returned before the instances it aborted had stopped, so the partition \
                 sweep it runs next races their last journal write"
            );
        });
    }

    /// An external abort cancels a supervisor at an await, so its own partition removal
    /// never runs. The prune sweeps the band below the cutoff by epoch number, which
    /// also collects what a previous process left behind.
    #[test]
    fn pruning_reclaims_the_journal_partitions_an_abort_left_behind() {
        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            // Epoch 3: aborted by an earlier prune (or a previous process) — the
            // leak. Epoch 5: the cutoff epoch's own live instance. Epoch 20: outside
            // the swept band, so a sweep that ignored the band would look identical.
            for epoch in [3u64, 5, 20] {
                ctx.open(&agreement_partition("", epoch), b"blob")
                    .await
                    .expect("partition");
            }
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            agreements.insert(
                Epoch::new(5),
                ctx.with_label("live")
                    .spawn(move |_| async move { std::future::pending::<()>().await }),
            );

            prune_agreements(&ctx, &mut agreements, 5, "", &mut 0).await;

            assert!(
                ctx.scan(&agreement_partition("", 3)).await.is_err(),
                "the partition an aborted supervisor left behind was never reclaimed"
            );
            assert!(
                ctx.scan(&agreement_partition("", 5)).await.is_ok(),
                "the live instance's own partition must survive"
            );
            assert!(
                ctx.scan(&agreement_partition("", 20)).await.is_ok(),
                "the sweep must stay inside its band"
            );
        });
    }

    /// The band's upper edge is exclusive: the cutoff epoch's own partition is not the
    /// sweep's business even when no instance for it is in the map, because after a
    /// restart that partition is the resumed voter's journal.
    #[test]
    fn the_sweep_leaves_the_cutoff_epochs_partition_alone_even_without_an_instance() {
        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            for epoch in [4u64, 5] {
                ctx.open(&agreement_partition("", epoch), b"blob")
                    .await
                    .expect("partition");
            }
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();

            prune_agreements(&ctx, &mut agreements, 5, "", &mut 0).await;

            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_err(),
                "the band below the cutoff must be swept, or this test saw no sweep"
            );
            assert!(
                ctx.scan(&agreement_partition("", 5)).await.is_ok(),
                "the cutoff epoch's partition was swept: a voter resumed for it after a \
                 restart would start from an empty journal"
            );
        });
    }

    /// A repeat prune at a cutoff already swept must not touch storage. A partition that
    /// appears under an already-swept cutoff is left alone until the cutoff moves, and
    /// the two edges that sweep are a new cutoff and an abort this call performed.
    #[test]
    fn a_repeat_prune_at_a_cutoff_already_swept_does_not_touch_storage() {
        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|ctx| async move {
            let mut agreements: BTreeMap<Epoch, Handle<()>> = BTreeMap::new();
            let mut swept_to = 0u64;

            // A fresh process: `swept_to = 0`, so the first prune sweeps and the leftover goes.
            ctx.open(&agreement_partition("", 4), b"blob")
                .await
                .expect("partition");
            prune_agreements(&ctx, &mut agreements, 5, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_err(),
                "the first prune at a cutoff this process has never been at must sweep"
            );
            assert_eq!(swept_to, 5, "the memo must name the cutoff just swept");

            // The same cutoff again, nothing aborted: the band is where it was.
            ctx.open(&agreement_partition("", 4), b"blob")
                .await
                .expect("partition");
            prune_agreements(&ctx, &mut agreements, 5, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_ok(),
                "the repeat prune swept the band again — `AGREEMENT_SWEEP_SPAN` filesystem \
                 removals on a call that moved nothing"
            );

            prune_agreements(&ctx, &mut agreements, 6, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 4)).await.is_err(),
                "a cutoff this process has never been at must sweep, or the gate is a leak"
            );
            assert_eq!(swept_to, 6);

            // A prune below the highest cutoff, with an instance to abort: it sweeps its own
            // band but must not lower the memo.
            ctx.open(&agreement_partition("", 1), b"blob")
                .await
                .expect("partition");
            agreements.insert(
                Epoch::new(1),
                ctx.with_label("stale")
                    .spawn(move |_| async move { std::future::pending::<()>().await }),
            );
            prune_agreements(&ctx, &mut agreements, 2, "", &mut swept_to).await;
            assert!(
                ctx.scan(&agreement_partition("", 1)).await.is_err(),
                "a prune that aborted an instance must sweep its band whatever the memo says"
            );
            assert_eq!(
                swept_to, 6,
                "the memo must be raised, never assigned: epoch 6's band is still swept"
            );
        });
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

    /// The two shapes a certificate can arrive in. A late body resolves through the
    /// subscribe; a body that arrived nowhere (the post-restart shape) ends the wait,
    /// because a supervisor that waited forever would never tear the instance down.
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

    /// The post-restart shape once the artifact store is durable: the rehydrated store
    /// answers the pairing outright, spending no part of the certification budget.
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
            assert!(store
                .insert(TARGET, (proposal.clone(), certificate.clone()))
                .is_ok());

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
            .map(|kp| build_signer(&ns, bimap.clone(), kp, TARGET, None).expect("member"))
            .collect();
        let proposal = Proposal::new(round, View::new(0), payload);
        let finalizes: Vec<_> = signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, proposal.clone()).expect("sign"))
            .collect();
        commonware_consensus::simplex::types::Finalization::from_finalizes(
            &build_verifier(&ns, bimap, TARGET, None),
            finalizes.iter(),
            &Sequential,
        )
        .expect("quorum")
    }

    /// A started four-member cohort on a simulated network, plus everything a test
    /// needs to steer it.
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
        /// Sign `seats`' confirmations of the full dealer-log set into the shared pool, the
        /// statement a proposer needs to clear the entry bar.
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
    /// confirmation pool is shared by all four, modelling a cohort that has already
    /// gossiped.
    async fn start_cohort(context: &deterministic::Context, seed: u64) -> Cohort {
        let (peers, bls) = signing_set(seed, N);
        let bimap = bimap_of(&peers, &bls);
        // The elector indexes the commonware-sorted participant set, not the order
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
                    partition_prefix: String::new(),
                    body_lost: None,
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

    /// Everything a [`Launcher`] under test needs that is not the thing under
    /// test: a started simulated network with one plane-owned broker per route,
    /// and this node's seat in a four-member committee.
    struct LauncherBench {
        keypair: ValidatorBlsKeypair,
        me: PeerPubkey,
        bimap: BiMap<PeerPubkey, BlsPubkey>,
        oracle: commonware_p2p::simulated::Oracle<PeerPubkey, deterministic::Context>,
        muxes: Vec<SimMux>,
    }

    type SimMux = SharedMux<
        commonware_p2p::simulated::Sender<PeerPubkey, deterministic::Context>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
    >;
    type SimLauncher = Launcher<
        deterministic::Context,
        commonware_p2p::simulated::Manager<PeerPubkey, deterministic::Context>,
        NoopResolver,
        commonware_p2p::simulated::Sender<PeerPubkey, deterministic::Context>,
        commonware_p2p::simulated::Receiver<PeerPubkey>,
    >;

    impl LauncherBench {
        async fn new(context: &deterministic::Context) -> Self {
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
            Self {
                keypair,
                me,
                bimap,
                oracle,
                muxes,
            }
        }

        /// The plane config over `committee`. The pinned-set seam is held and
        /// never read: `derive` parks, which is the honest local state for a
        /// node whose ceremony cannot answer.
        fn config(
            &self,
            committee: CommitteeSource,
            pinned_tx: mpsc::Sender<PinnedRequest>,
            probe: Option<watch::Sender<LauncherProbe>>,
        ) -> AgreementPlaneConfig<
            commonware_p2p::simulated::Manager<PeerPubkey, deterministic::Context>,
            NoopResolver,
        > {
            AgreementPlaneConfig {
                chain_id: CHAIN_ID,
                keypair: self.keypair.clone(),
                me: self.me.clone(),
                peers: self.oracle.manager(),
                logs: NoopResolver,
                recorded: full_index(),
                pinned_requests: pinned_tx,
                confirms: ConfirmPool::new(b"FLUENT_TEST_LAUNCHER"),
                metrics: BeaconMetrics::default(),
                artifacts: ArtifactStore::new(),
                committee,
                mailbox_size: 64,
                partition_prefix: String::new(),
                body_lost: None,
                timeouts: AgreementTimeouts {
                    leader: Duration::from_secs(2),
                    certification: Duration::from_secs(3),
                    timeout_retry: Duration::from_millis(500),
                    fetch: Duration::from_millis(500),
                    ..AgreementTimeouts::coarse()
                },
                probe,
            }
        }

        fn muxes(
            &self,
        ) -> AgreementMuxes<
            commonware_p2p::simulated::Sender<PeerPubkey, deterministic::Context>,
            commonware_p2p::simulated::Receiver<PeerPubkey>,
        > {
            AgreementMuxes {
                vote: self.muxes[0].clone(),
                cert: self.muxes[1].clone(),
                resolver: self.muxes[2].clone(),
                bodies: self.muxes[3].clone(),
            }
        }

        /// A launcher over `committee`, with the node's latch `halt`, driven by
        /// the test directly (`on_request` / `on_tick`).
        fn launcher(
            &self,
            context: &deterministic::Context,
            committee: CommitteeSource,
            halt: SafetyHalt,
        ) -> (SimLauncher, mpsc::Sender<PinnedRequest>) {
            let (out_tx, _out_rx) = mpsc::channel(4);
            let (pinned_tx, _pinned_rx) = mpsc::channel(8);
            let launcher = Launcher::new(
                context.with_label("launcher"),
                self.config(committee, pinned_tx.clone(), None),
                self.muxes(),
                out_tx,
                halt,
            );
            (launcher, pinned_tx)
        }

        /// The real launcher task ([`spawn_agreement_launcher`]) over `committee`
        /// and `halt`, with the test holding its three inputs — the request
        /// stream, the epoch clock, the latch — and its probe.
        fn task(
            &self,
            context: &deterministic::Context,
            committee: CommitteeSource,
            halt: SafetyHalt,
        ) -> LauncherTask {
            let (out_tx, _out_rx) = mpsc::channel(4);
            let (pinned_tx, _pinned_rx) = mpsc::channel(8);
            let (requests_tx, requests_rx) = mpsc::channel(8);
            let (clock_tx, clock_rx) = watch::channel(0u64);
            let (probe_tx, probe_rx) = watch::channel(LauncherProbe::default());
            let handle = spawn_agreement_launcher(
                context.with_label("launcher"),
                self.config(committee, pinned_tx.clone(), Some(probe_tx)),
                self.muxes(),
                requests_rx,
                out_tx,
                clock_rx,
                halt,
            );
            LauncherTask {
                handle,
                requests: requests_tx,
                clock: clock_tx,
                probe: probe_rx,
                _pinned: pinned_tx,
                _out: _out_rx,
            }
        }

        /// Whether the instance for `target` still holds its vote route on the
        /// plane's mux. A duplicate `register` is refused while it does
        /// (`p2p/src/utils/mux.rs:127-129`), and the route is deregistered when
        /// the instance's `SubReceiver` drops (`:270-282`) — so this is the
        /// instance's own footprint, not the launcher's bookkeeping.
        async fn instance_holds_its_route(&self, target: u64) -> bool {
            match register_dkg_subchannel(&self.muxes[0], target).await {
                // Ours to drop: the route deregisters again with it.
                Ok(_route) => false,
                Err(_) => true,
            }
        }

        async fn wait_until_route_is_free(&self, context: &deterministic::Context, target: u64) {
            for _ in 0..2_000 {
                if !self.instance_holds_its_route(target).await {
                    return;
                }
                context.sleep(Duration::from_millis(10)).await;
            }
            panic!("the instance for {target} still holds its vote route");
        }
    }

    /// A running launcher task and the test's ends of its inputs.
    struct LauncherTask {
        handle: Handle<()>,
        requests: mpsc::Sender<u64>,
        clock: watch::Sender<u64>,
        probe: watch::Receiver<LauncherProbe>,
        _pinned: mpsc::Sender<PinnedRequest>,
        _out: mpsc::Receiver<AgreedArtifact>,
    }

    impl LauncherTask {
        /// The task's state once it has handled at least `events` events.
        async fn after(&mut self, events: u64) -> LauncherProbe {
            self.probe
                .wait_for(|p| p.events >= events)
                .await
                .expect("the launcher task dropped its probe")
                .clone()
        }
    }

    /// The targets a launcher holds a running instance for.
    fn running<E, P, R, HS, HR>(launcher: &Launcher<E, P, R, HS, HR>) -> Vec<u64>
    where
        HS: Sender<PublicKey = PeerPubkey>,
        HR: Receiver<PublicKey = PeerPubkey>,
    {
        launcher.instances.keys().map(|e| e.get()).collect()
    }

    /// The launcher turns the beacon actor's dealing-closed edge into a running
    /// instance. A repeat for a running target is a no-op, and a request the plane could
    /// not act on — an unreadable `committee[epoch]` — leaves the target retryable.
    #[test]
    fn the_launcher_starts_one_instance_per_target_and_retries_an_unreadable_committee() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;

            let readable = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                let readable = readable.clone();
                let reads = reads.clone();
                Arc::new(move |epoch: u64| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if epoch != TARGET && !readable.load(std::sync::atomic::Ordering::Relaxed) {
                        return None;
                    }
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            let (mut launcher, _pinned) = bench.launcher(&context, committee, halt);

            launcher.on_request(TARGET).await;
            assert_eq!(running(&launcher), vec![TARGET]);
            assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);

            launcher.on_request(TARGET).await;
            assert_eq!(running(&launcher), vec![TARGET]);
            assert_eq!(
                reads.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "a repeat for a running target must be settled before the committee read"
            );
            // An unreadable committee must not consume the target.
            launcher.on_request(TARGET + 1).await;
            assert_eq!(
                running(&launcher),
                vec![TARGET],
                "an unreadable committee started an instance"
            );
            assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2);

            readable.store(true, std::sync::atomic::Ordering::Relaxed);
            launcher.on_request(TARGET + 1).await;
            assert_eq!(
                running(&launcher),
                vec![TARGET, TARGET + 1],
                "a target whose committee became readable must still get its instance"
            );

            launcher.abort_all("test over");
        });
    }

    /// A halted node starts no agreement instance, and the instances live when the
    /// latch engages are aborted on its edge. The refusal is before the spawn, and it
    /// settles the target so the actor's re-announcement is not a per-tick warning.
    #[test]
    fn a_halted_node_starts_no_agreement_instance_and_aborts_the_ones_it_has() {
        use std::{
            future::Future as _,
            pin::Pin,
            task::{Context as TaskContext, Poll},
        };

        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                let reads = reads.clone();
                Arc::new(move |epoch: u64| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            let (mut launcher, _pinned) = bench.launcher(&context, committee, halt.clone());

            launcher.on_request(TARGET).await;
            launcher.on_tick(TARGET - 1).await;
            assert_eq!(running(&launcher), vec![TARGET]);

            halt.engage(crate::sync_metrics::SyncReason::ResultDivergence);

            let reads_before = reads.load(std::sync::atomic::Ordering::SeqCst);
            launcher.on_request(TARGET + 1).await;
            launcher.on_request(TARGET + 1).await;
            assert_eq!(
                running(&launcher),
                vec![TARGET],
                "a halted node started an epoch-key agreement instance"
            );
            assert_eq!(
                reads.load(std::sync::atomic::Ordering::SeqCst),
                reads_before,
                "the refusal must sit before the committee read"
            );
            assert!(
                launcher.started.contains(&(TARGET + 1)),
                "a refused target must be settled, or the actor's re-announcement warns per tick"
            );

            let mut handle = launcher
                .instances
                .remove(&Epoch::new(TARGET))
                .expect("the running instance");
            let waker = futures::task::noop_waker();
            let mut cx = TaskContext::from_waker(&waker);
            assert!(
                matches!(Pin::new(&mut handle).poll(&mut cx), Poll::Pending),
                "the instance must still be running before the halt edge"
            );
            launcher.instances.insert(Epoch::new(TARGET), handle);
            launcher.on_tick(TARGET - 1).await;
            assert_eq!(
                running(&launcher),
                vec![TARGET],
                "a clock tick below the target is not the halt"
            );
            launcher.on_halt().await;
            assert!(
                running(&launcher).is_empty(),
                "the instances live at the halt must be aborted on the halt edge"
            );
            assert!(
                !bench.instance_holds_its_route(TARGET).await,
                "the aborted instance was not joined: its vote route is still registered"
            );
        });
    }

    /// The latch can engage during a spawn: `start_one` awaits the four mux
    /// registrations, so the instance just started is retired on the same `on_request`
    /// before the launcher does anything else.
    #[test]
    fn a_latch_that_engages_during_the_spawn_retires_the_instance_it_started() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                let reads = reads.clone();
                Arc::new(move |epoch: u64| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            let (mut launcher, _pinned) = bench.launcher(&context, committee, halt.clone());

            let guard = bench.muxes[0].clone().lock_owned().await;
            context.with_label("flip").spawn({
                let halt = halt.clone();
                move |ctx| async move {
                    ctx.sleep(Duration::from_secs(1)).await;
                    halt.engage(crate::sync_metrics::SyncReason::ResultDivergence);
                    drop(guard);
                }
            });
            launcher.on_request(TARGET).await;

            assert_eq!(
                reads.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "the request must have gone past the latch check into the spawn"
            );
            assert!(
                running(&launcher).is_empty(),
                "an instance spawned under a latch that engaged during its registrations \
                 survived the request that started it"
            );
            assert!(
                launcher.started.contains(&TARGET),
                "the target stays settled: nothing retries a halted node's spawn"
            );
            assert!(
                !bench.instance_holds_its_route(TARGET).await,
                "the retired instance was not joined: its vote route is still registered"
            );
        });
    }

    /// The launcher task end to end: a request starts an instance the task holds, the
    /// clock edge prunes it, and the request stream closing exits the task and takes
    /// its instances with it.
    #[test]
    fn the_launcher_task_starts_on_a_request_prunes_on_the_clock_edge_and_aborts_on_exit() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                Arc::new(move |epoch: u64| {
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            let mut task = bench.task(&context, committee, halt);

            task.requests.send(TARGET).await.expect("task alive");
            let state = task.after(1).await;
            assert_eq!(
                state.instances,
                vec![TARGET],
                "the request did not start an instance"
            );
            assert!(bench.instance_holds_its_route(TARGET).await);

            // The clock enters the epoch after the target: pruned, and joined.
            task.clock.send_replace(TARGET + 1);
            let state = task.after(2).await;
            assert!(
                state.instances.is_empty(),
                "the clock edge past the target did not prune: {state:?}"
            );
            assert!(
                !bench.instance_holds_its_route(TARGET).await,
                "the pruned instance was not joined: its vote route is still registered"
            );

            // A later target, then the actor goes away: the task exits and the
            // instance it still held is gone with it.
            task.requests.send(TARGET + 2).await.expect("task alive");
            let state = task.after(3).await;
            assert_eq!(state.instances, vec![TARGET + 2]);
            assert!(bench.instance_holds_its_route(TARGET + 2).await);

            let LauncherTask {
                handle, requests, ..
            } = task;
            drop(requests);
            handle
                .await
                .expect("the launcher task must exit cleanly once the request stream closes");
            bench.wait_until_route_is_free(&context, TARGET + 2).await;
        });
    }

    /// The halt edge reaches the launcher task on its own: with the clock silent, the
    /// latch engaging aborts and joins the running instance, and the task then keeps
    /// answering requests. The `watch` edge resolves on every call once engaged, so the
    /// arm is disarmed after its first firing.
    #[test]
    fn a_halt_edge_aborts_the_running_instance_without_a_clock_tick_and_the_task_keeps_answering() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                let reads = reads.clone();
                Arc::new(move |epoch: u64| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            let mut task = bench.task(&context, committee, halt.clone());

            task.requests.send(TARGET).await.expect("task alive");
            let state = task.after(1).await;
            assert_eq!(state.instances, vec![TARGET]);
            assert!(bench.instance_holds_its_route(TARGET).await);
            assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);

            // The latch engages. No clock edge, no request.
            halt.engage(crate::sync_metrics::SyncReason::ResultDivergence);
            let state = task.after(2).await;
            assert!(
                state.instances.is_empty(),
                "the halt edge did not abort the running instance: {state:?}"
            );
            assert!(
                !bench.instance_holds_its_route(TARGET).await,
                "the halted instance was not joined: its vote route is still registered"
            );

            // Silence: an armed-again edge would be handled once per iteration.
            context.sleep(Duration::from_secs(10)).await;
            assert_eq!(
                task.probe.borrow().events,
                2,
                "the launcher task handled events with no input: the halt arm is not disarmed"
            );

            // Still answering: the next request is refused before the committee
            // read, and the task is the one that says so.
            task.requests.send(TARGET + 1).await.expect("task alive");
            let state = task.after(3).await;
            assert_eq!(state.events, 3);
            assert!(
                state.instances.is_empty(),
                "a halted node started an instance"
            );
            assert_eq!(
                reads.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "the refusal must sit before the committee read"
            );
            assert!(!bench.instance_holds_its_route(TARGET + 1).await);
        });
    }

    /// A latch engaged before the launcher is built refuses the first request: no
    /// spawn, no committee read. The already-high `watch` edge fires on the first
    /// select with nothing to abort, and is disarmed.
    #[test]
    fn a_latch_engaged_before_the_launcher_is_built_refuses_the_first_request() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let bench = LauncherBench::new(&context).await;
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let committee: CommitteeSource = {
                let bimap = bench.bimap.clone();
                let reads = reads.clone();
                Arc::new(move |epoch: u64| {
                    reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Some(EpochCommittee::from_unverified(epoch, bimap.clone()))
                })
            };
            let halt = SafetyHalt::new(crate::sync_metrics::SyncMetrics::default());
            halt.engage(crate::sync_metrics::SyncReason::ElInvalid);
            let mut task = bench.task(&context, committee, halt);

            task.requests.send(TARGET).await.expect("task alive");
            let state = task.after(2).await;
            assert_eq!(state.events, 2);
            assert!(
                state.instances.is_empty(),
                "a halted node started an instance"
            );
            assert_eq!(
                reads.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "a restored latch must refuse before the committee read"
            );
            assert!(!bench.instance_holds_its_route(TARGET).await);

            context.sleep(Duration::from_secs(10)).await;
            assert_eq!(
                task.probe.borrow().events,
                2,
                "the halt arm is not disarmed"
            );
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

            // Every member kept the artifact it agreed, so a peer asking any of them is
            // served. Each verifies against `committee[TARGET]` alone.
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
                        .scan(&agreement_partition("", epoch))
                        .await
                        .map(|blobs| blobs.is_empty())
                        .unwrap_or(true),
                    "the agreement journal partition survived the teardown"
                );
            }
        });
    }

    /// Below the entry bar the plane never aborts: it nullifies view after view and
    /// keeps every instance alive, so the epoch is agreed the moment enough members
    /// confirm. The second half makes the first an assertion rather than a timeout.
    #[test]
    fn the_plane_waits_below_the_entry_bar_and_never_aborts() {
        let runner = deterministic::Runner::timed(Duration::from_secs(3600));
        runner.start(|context| async move {
            let mut cohort = start_cohort(&context, 5).await;
            let bar = entry_bar(N, View::new(1));
            cohort.confirm(0..bar - 1);
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

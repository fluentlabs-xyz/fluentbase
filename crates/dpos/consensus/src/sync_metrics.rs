//! Self-heal observability — the `dpos_sync_degraded{reason}` stuck-detector.
//!
//! The reth-aligned recovery posture keeps a node UP and retrying (rather than
//! `process::exit`-ing) wherever fork-safety permits; the gauge, not a crash, is
//! then the operator's stuck signal. A [`SyncMetrics`] is created + registered
//! ONCE per launch (mirrors [`crate::beacon::metrics::BeaconMetrics`]) against
//! the launch context (commonware `Metrics`, scraped at `:19100` — NOT the
//! `metrics::` macro recorder) and cloned into the cold-start + boundary-hook
//! self-heal loops. Each metric is `Arc`-backed, so the struct is cheap to clone
//! and every clone shares one counter.
//!
//! Contract (modeled on the executor's `deferred_height`): a `reason` held at 1
//! for `>Xm` is the alertable stuck-node signal that replaces the removed fatal.

use commonware_runtime::Metrics;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue, LabelValueEncoder},
    metrics::{counter::Counter, family::Family, gauge::Gauge},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};
use tokio::sync::Notify;
use tracing::{error, warn};

/// Why a node is self-healing rather than participating normally — the single
/// bounded label set of `dpos_sync_degraded`. Set to 1 while the matching
/// self-heal loop retries; cleared to 0 on recovery. Variant numbering follows
/// the recovery-taxonomy path ids (research T1).
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SyncReason {
    /// #11 cold-start EL-sync found zero devp2p peers — re-attempting forever.
    NoPeers,
    /// #13 reth does not yet hold the DPoS activation block — polling forever.
    ActivationWait,
    /// #4 cold-start has no upstream/frontier to authenticate the anchor yet.
    AwaitingUpstream,
    /// #1 a jump target failed committee-BLS; rotating/backing-off upstreams.
    AuthRotate,
    /// #14 a transient engine-API transport error; retrying the FCU/import.
    EngineRetry,
    /// #17 a just-landed block is not yet reth-visible; visibility belt retrying.
    LandingWait,
    /// #8/#12 crash-survivor recovery deferred to devp2p / by-height re-fetch.
    CrashRecover,
    /// #16 the epoch-boundary staking hook keeps erroring; retrying-degraded.
    BoundaryHook,
    /// The EL did not make a finalized derived block canonical (dropped import /
    /// SYNCING FCU); re-applying (re-derive + import + FCU) until it lands.
    FinalizeApply,
    /// #2/#3 SafetyHalt: attested result diverged from local execution.
    ResultDivergence,
    /// #10 SafetyHalt: synced head does not descend from the L1-finalized root.
    L1Fork,
    /// #15 SafetyHalt: reth returned Invalid for our locally-derived block.
    ElInvalid,
}

impl SyncReason {
    /// The `reason` label value — snake_case, matching the alert/dashboard names.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoPeers => "no_peers",
            Self::ActivationWait => "activation_wait",
            Self::AwaitingUpstream => "awaiting_upstream",
            Self::AuthRotate => "auth_rotate",
            Self::EngineRetry => "engine_retry",
            Self::LandingWait => "landing_wait",
            Self::CrashRecover => "crash_recover",
            Self::BoundaryHook => "boundary_hook",
            Self::FinalizeApply => "finalize_apply",
            Self::ResultDivergence => "result_divergence",
            Self::L1Fork => "l1_fork",
            Self::ElInvalid => "el_invalid",
        }
    }

    /// Parse a label produced by [`Self::as_str`] — the inverse used to reload a
    /// persisted [`SafetyHalt`] marker. `None` for a label this build does not
    /// know (a marker written by another version).
    pub fn from_label(label: &str) -> Option<Self> {
        const ALL: [SyncReason; 12] = [
            SyncReason::NoPeers,
            SyncReason::ActivationWait,
            SyncReason::AwaitingUpstream,
            SyncReason::AuthRotate,
            SyncReason::EngineRetry,
            SyncReason::LandingWait,
            SyncReason::CrashRecover,
            SyncReason::BoundaryHook,
            SyncReason::FinalizeApply,
            SyncReason::ResultDivergence,
            SyncReason::L1Fork,
            SyncReason::ElInvalid,
        ];
        ALL.into_iter().find(|r| r.as_str() == label)
    }
}

impl EncodeLabelValue for SyncReason {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
        EncodeLabelValue::encode(&self.as_str(), encoder)
    }
}

/// The `{reason=...}` label set of the `dpos_sync_degraded` gauge family.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DegradedLabels {
    reason: SyncReason,
}

/// Self-heal counters + the labeled stuck-detector gauge. See the module docs for
/// the registration + clone topology.
#[derive(Clone, Debug, Default)]
pub struct SyncMetrics {
    /// `dpos_sync_degraded{reason}` — 1 while the `reason` self-heal loop retries.
    degraded: Family<DegradedLabels, Gauge<i64>>,
    /// #14 transient engine-API (FCU/import) transport errors retried, not exited.
    pub engine_transient_retry: Counter,
    /// #8/#12 crash-survivor recoveries deferred to devp2p EL-sync / by-height re-fetch.
    pub crash_recover_deferred_to_elsync: Counter,
    /// #12 block distance reth is behind its own consensus archive (0 = none).
    pub crash_recover_gap_blocks: Gauge<i64>,
    /// #8 below-floor marshal-archive holes healed by a BLS-verified by-height
    /// re-fetch through the cert upstream (the live marshal resolver cannot repair
    /// below its finalized floor, so the crash-survivor recovery re-fetches inline).
    pub crash_recover_refetched: Counter,
    /// Epoch-boundary blocks seeded below the marshal floor so a jumped member can
    /// spawn its engine in the LANDING epoch instead of parking verify-only until the
    /// next boundary. Bumped at the INJECTION sites, once per height actually stored — a
    /// successful fetch that the both-or-neither batch then discards leaves this flat, so
    /// the counter can never read non-zero while nothing was injected.
    pub jump_boundary_refetched: Counter,
    /// Boundary seeding that failed (upstream absent/silent, wrong height served,
    /// structural or BLS check failed, committee unreadable). Non-zero means a member
    /// stayed verify-only for its landing epoch — no proposals, no votes — which is
    /// the pre-fix behaviour, degraded to loudly instead of silently.
    pub jump_boundary_refetch_failed: Counter,
    /// A steady-state re-jump aborted because reth was CONNECTED but its executed
    /// head stayed frozen for the stall window — the soak-v43 connected-but-wedged
    /// EL pipeline. Bumped once per re-jump attempt that trips the stall net; a
    /// climbing value against a plateaued chain height is the alertable signal that
    /// this node is deterministically re-wedging (the divergence root cause is
    /// unknown, so the node stays observable + deferred rather than silently stuck).
    pub el_sync_stalled_with_peers: Counter,
    /// `dpos_safety_halt_engaged` — 1 once the fork-safety latch is engaged, in
    /// THIS process or in a previous one (a restart reloads the datadir marker
    /// and re-engages before the first event). Separate from
    /// `dpos_sync_degraded{reason}` because the halt is permanent and
    /// operator-cleared while a degraded reason is a self-heal loop that clears
    /// itself — and because a marker written by another build carries a reason
    /// label this build cannot decode, leaving the labeled gauge silent.
    pub safety_halt_engaged: Gauge<i64>,
}

impl SyncMetrics {
    /// Register every metric on the commonware registry. Call once, against the
    /// launch context (mirrors `beacon/metrics.rs::BeaconMetrics::register`).
    pub fn register(&self, ctx: &impl Metrics) {
        ctx.register(
            "dpos_sync_degraded",
            "1 while a self-heal loop is retrying for the labeled `reason` (0 = healthy). \
             A reason held at 1 for > Xm is the alertable stuck-node signal that replaces \
             the removed process::exit.",
            self.degraded.clone(),
        );
        ctx.register(
            "engine_transient_retry_total",
            "Transient engine-API (FCU/import) transport errors retried without exiting.",
            self.engine_transient_retry.clone(),
        );
        ctx.register(
            "crash_recover_deferred_to_elsync_total",
            "Crash-survivor recoveries deferred to devp2p EL-sync / by-height re-fetch.",
            self.crash_recover_deferred_to_elsync.clone(),
        );
        ctx.register(
            "crash_recover_gap_blocks",
            "Blocks reth is behind its own consensus archive during crash-survivor recovery \
             (0 = none).",
            self.crash_recover_gap_blocks.clone(),
        );
        ctx.register(
            "crash_recover_refetched_total",
            "Below-floor marshal-archive holes healed by a BLS-verified by-height re-fetch \
             through the cert upstream during crash-survivor recovery.",
            self.crash_recover_refetched.clone(),
        );
        ctx.register(
            "dpos_jump_boundary_refetched_total",
            "Epoch-boundary blocks seeded below the marshal floor after a jump, so the member \
             can spawn its engine in the landing epoch.",
            self.jump_boundary_refetched.clone(),
        );
        ctx.register(
            "dpos_jump_boundary_refetch_failed_total",
            "Boundary seeding attempts that failed — the member stays verify-only (no proposals, \
             no votes) for its landing epoch.",
            self.jump_boundary_refetch_failed.clone(),
        );
        ctx.register(
            "el_sync_stalled_with_peers_total",
            "Steady-state re-jumps aborted because reth was connected but its executed head \
             stayed frozen for the stall window (the connected-but-wedged EL pipeline). A \
             climbing value against a plateaued chain height means this node is \
             deterministically re-wedging on EL-sync.",
            self.el_sync_stalled_with_peers.clone(),
        );
        ctx.register(
            "dpos_safety_halt_engaged",
            "1 once the fork-safety latch is engaged (this process or a previous one, via the              datadir marker). Permanent: it is cleared by an operator removing the marker after              the fork is resolved on L1, never by the node itself. A node reporting 1 signs,              proposes and votes on nothing.",
            self.safety_halt_engaged.clone(),
        );
    }

    /// Mark the `reason` self-heal loop as active (`dpos_sync_degraded{reason}=1`).
    pub fn degrade(&self, reason: SyncReason) {
        self.degraded
            .get_or_create(&DegradedLabels { reason })
            .set(1);
    }

    /// Clear the `reason` (`dpos_sync_degraded{reason}=0`) — the loop recovered.
    pub fn recover(&self, reason: SyncReason) {
        self.degraded
            .get_or_create(&DegradedLabels { reason })
            .set(0);
    }

    /// Current `dpos_sync_degraded{reason}` value (test/assert helper).
    pub fn degraded_value(&self, reason: SyncReason) -> i64 {
        self.degraded
            .get_or_create(&DegradedLabels { reason })
            .get()
    }
}

/// The two clocks the DPoS node runs on, published side by side.
///
/// The ordering plane and the execution pipeline are separate subsystems, but
/// the beacon plane's clock is derived from the EXECUTION one: the finalized
/// poller in `fluentbase-node`'s `dpos.rs` reads `finalized_block_number()` and
/// feeds `fin + K` to the DkgActor and the `EpochTransition`. So every way the
/// executor can stop — the `SafetyHalt` park, an unfillable `Corruption`, a
/// wedge nobody has found yet — also stops DKG ceremonies and epoch boundaries,
/// and did so with no series to show it.
///
/// The ordering half comes off marshal's `Update::Tip` in `FluentApp::report`,
/// which is BFT-attested and INDEPENDENT of execution — and `FluentApp` is built
/// once per process, so it outlives a halt and every engine abort. That is the
/// whole point of the pair: the ordering gauge keeps climbing while the
/// execution one freezes, and `dpos_dkg_clock_lag_blocks` is exactly the gap
/// between "the committee is still agreeing" and "this node stopped executing".
///
/// Registered by the plane builder (the node crate, where the poller lives),
/// mirroring [`SyncMetrics`]'s clone-shares-one-gauge topology.
#[derive(Clone, Debug, Default)]
pub struct PlaneClock {
    ordering: Gauge<i64>,
    dkg: Gauge<i64>,
    lag: Gauge<i64>,
}

impl PlaneClock {
    /// Register on the commonware registry. Call once, against the launch
    /// context. A `PlaneClock` that is never registered publishes nothing — the
    /// honest state for a node that runs no DKG clock at all.
    pub fn register(&self, ctx: &impl Metrics) {
        ctx.register(
            "dpos_ordering_finalized_height",
            "Marshal's BFT-attested ordering finalization tip. Advances on committee agreement \
             alone — it does not wait for this node to execute anything.",
            self.ordering.clone(),
        );
        ctx.register(
            "dpos_dkg_clock_height",
            "The ordering height the beacon plane's clock has reached: the EL finalized block \
             number plus K, as fed to the DkgActor and the EpochTransition. Frozen ⇒ no DKG \
             ceremony progress and no epoch boundary detection, whatever the ordering plane does.",
            self.dkg.clone(),
        );
        ctx.register(
            "dpos_dkg_clock_lag_blocks",
            "dpos_ordering_finalized_height − dpos_dkg_clock_height, floored at 0. Bounded and \
             flat = execution is keeping up; growing at the block rate = execution has stopped \
             while the committee keeps finalizing without this node.",
            self.lag.clone(),
        );
    }

    /// Marshal reported a new BFT-attested ordering finalization tip.
    pub fn record_ordering_tip(&self, height: u64) {
        self.ordering.set(height as i64);
        self.refresh_lag();
    }

    /// The finalized poller fed the beacon plane a new clock height (`fin + K`).
    pub fn record_dkg_clock(&self, height: u64) {
        self.dkg.set(height as i64);
        self.refresh_lag();
    }

    /// Floored at 0 because the two gauges are written by different tasks: the
    /// DKG clock legitimately reads one block ahead of the ordering tip between
    /// the two writes, and a negative lag would render as a spike rather than as
    /// the "nothing to report" it is.
    fn refresh_lag(&self) {
        self.lag.set((self.ordering.get() - self.dkg.get()).max(0));
    }

    /// Current `(ordering, dkg, lag)` — test/assert helper.
    pub fn snapshot(&self) -> (i64, i64, i64) {
        (self.ordering.get(), self.dkg.get(), self.lag.get())
    }
}

/// Fork-safety latch (Phase 3 `SafetyHalt`). A node that detects it would extend
/// a branch honest peers reject — #2/#3 result divergence, #15 an EL `Invalid`
/// verdict on a locally-derived block, #10 an L1-fork (`holds()==false`) after an
/// EL-sync jump — HALTS instead of `process::exit`-ing OR (worse) trusting the
/// cert and continuing. Engaging it:
///
/// 1. raises `dpos_sync_degraded{reason}=1` (`result_divergence` / `el_invalid` /
///    `l1_fork`) — the alertable "this node refuses the chain" signal;
/// 2. LATCHES so [`crate::epoch_manager::Actor::reconcile_roles`] never
///    (re-)promotes the node to a participating `Signer` — demoted to verify-only
///    permanently (stop signing/proposing/voting);
/// 3. fires a one-shot [`Notify`] so the epoch manager aborts any running engine
///    immediately (not just at the next boundary).
///
/// The executor then stops driving reth forward, and the OuterEngine supervisor
/// keeps marshal + `consensus`-RPC alive (it does NOT abort-all — that is
/// reserved for a genuine subsystem CRASH) so the node stays observable and can
/// be recovered by the L1 SP1 validity proof + social/governance action. It is
/// a permanent latch: there is deliberately no `disengage` — recovery is
/// external (a fresh, re-synced start after the fork is resolved on L1).
///
/// "Permanent" has to survive a process restart, so engaging also writes a
/// one-line marker into the datadir ([`Self::restoring`]). Without it a
/// `docker restart` silently cleared a latch that has no in-process
/// `disengage`: the node came back a full signer, on the same disk, with no
/// record of what it had refused. The marker is cleared by an OPERATOR deleting
/// the file — never by the node. Automatic recovery is deliberately absent:
/// neither arming class is separable from network evidence with the data the
/// node holds at arming time, and "wipe local state and resync" after network
/// evidence destroys the only thing that distinguishes this node's view from
/// the disputed quorum certificate.
///
/// Arc-backed → cheap to clone; every clone shares one latch + one gauge family.
#[derive(Clone, Default)]
pub struct SafetyHalt {
    engaged: Arc<AtomicBool>,
    /// The verdict that engaged the latch — the FIRST one wins (a later engage
    /// is a consequence of the first, not a second diagnosis). Typed rather
    /// than reconstructed from a prometheus label or an eyre display string, so
    /// the restart gate and the operator log read the same value the arming
    /// site decided.
    reason: Arc<OnceLock<SyncReason>>,
    /// Datadir marker path. `None` for in-process / test latches, which have no
    /// datadir and must not write one.
    marker: Option<Arc<PathBuf>>,
    notify: Arc<Notify>,
    metrics: SyncMetrics,
}

impl SafetyHalt {
    /// Build a latch that raises its `reason` gauge on the SHARED (already
    /// registered) [`SyncMetrics`] from `dpos.rs::launch`. No datadir marker:
    /// for in-process and test use only.
    pub fn new(metrics: SyncMetrics) -> Self {
        Self {
            engaged: Arc::default(),
            reason: Arc::default(),
            marker: None,
            notify: Arc::default(),
            metrics,
        }
    }

    /// Build the production latch: bound to a datadir `marker` path, and
    /// ALREADY ENGAGED when a previous run of this node left one behind.
    ///
    /// This is the restart gate. A restored latch raises
    /// `dpos_safety_halt_engaged` + `dpos_sync_degraded{reason}` and logs the
    /// reason BEFORE the first consensus event, and
    /// `epoch_manager::reconcile_roles` reads `is_engaged()` when deciding
    /// membership — so the node comes up permanently verify-only (signs
    /// nothing, proposes nothing, votes on nothing) until an operator removes
    /// the file.
    ///
    /// A marker this build cannot decode still latches: the file's EXISTENCE is
    /// the halt record; its content only names the reason.
    pub fn restoring(metrics: SyncMetrics, marker: PathBuf) -> Self {
        let halt = Self {
            engaged: Arc::default(),
            reason: Arc::default(),
            marker: Some(Arc::new(marker)),
            notify: Arc::default(),
            metrics,
        };
        halt.restore_marker();
        halt
    }

    /// Re-engage from an existing datadir marker, if there is one.
    fn restore_marker(&self) {
        let Some(path) = self.marker.as_deref() else {
            return;
        };
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            // A missing marker is the normal, healthy start.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                // Present-but-unreadable: latch, because a marker we cannot read
                // is not evidence that there is none.
                error!(
                    marker = %path.display(),
                    %error,
                    "SafetyHalt marker exists but could not be read — coming up HALTED \
                     (verify-only). Resolve the fork on L1, then delete the marker to \
                     restore this node."
                );
                self.latch();
                return;
            }
        };
        let label = raw.trim();
        match SyncReason::from_label(label) {
            Some(reason) => {
                error!(
                    marker = %path.display(),
                    reason = label,
                    "node was SafetyHalted in a previous run — coming up HALTED (verify-only): \
                     no signing, no proposing, no voting. Recovery is the L1 SP1 validity proof \
                     + governance, then a fresh re-synced start; delete the marker to clear."
                );
                self.engage(reason);
            }
            None => {
                error!(
                    marker = %path.display(),
                    content = label,
                    "SafetyHalt marker holds an unrecognised reason (written by another build) \
                     — coming up HALTED (verify-only) anyway; the marker's existence is the \
                     halt record. Delete it to clear."
                );
                self.latch();
            }
        }
    }

    /// Set the latch bit + the unlabeled gauge and fire the one-shot edge.
    /// Shared by [`Self::engage`] and the reason-less restore paths.
    fn latch(&self) {
        self.metrics.safety_halt_engaged.set(1);
        if !self.engaged.swap(true, Ordering::SeqCst) {
            self.notify.notify_one();
        }
    }

    /// Latch the halt + raise `dpos_sync_degraded{reason}=1`, and persist the
    /// reason to the datadir marker so a restart re-engages instead of silently
    /// clearing. Idempotent; the epoch-manager wakeup fires once, on the 0→1
    /// edge, and the FIRST reason is the one recorded.
    pub fn engage(&self, reason: SyncReason) {
        self.metrics.degrade(reason);
        let first = self.reason.set(reason).is_ok();
        self.latch();
        // Only the first verdict is persisted: a later engage is downstream of
        // it, and rewriting would replace the diagnosis with its consequence.
        if first {
            self.persist_marker(reason);
        }
    }

    /// Best-effort marker write. A failure NEVER fails the halt — the in-process
    /// latch already holds — but it is loud, because it means this node WILL
    /// come back as a signer after a restart.
    fn persist_marker(&self, reason: SyncReason) {
        let Some(path) = self.marker.as_deref() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                warn!(dir = %parent.display(), %error, "could not create the SafetyHalt marker directory");
            }
        }
        match std::fs::write(path, format!("{}\n", reason.as_str())) {
            Ok(()) => error!(
                marker = %path.display(),
                reason = reason.as_str(),
                "SafetyHalt marker written — this node stays verify-only across restarts until \
                 an operator deletes the marker"
            ),
            Err(error) => error!(
                marker = %path.display(),
                %error,
                reason = reason.as_str(),
                "could not persist the SafetyHalt marker — a restart of this node WILL clear \
                 the latch and it will sign again; write the marker by hand or keep the node down"
            ),
        }
    }

    /// The verdict that engaged the latch, `None` while healthy (or when the
    /// latch was restored from a marker this build cannot decode).
    pub fn reason(&self) -> Option<SyncReason> {
        self.reason.get().copied()
    }

    /// The datadir marker path an operator must delete to clear this latch.
    pub fn marker_path(&self) -> Option<&Path> {
        self.marker.as_deref().map(PathBuf::as_path)
    }

    /// Whether the node is safety-halted — read by `reconcile_roles` (never
    /// re-promote) and by the OuterEngine supervisor (park instead of abort-all).
    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }

    /// Await the 0→1 engage edge (the epoch-manager select arm that aborts the
    /// running engine). `notify_one` stores a permit, so an engage that races
    /// ahead of the waiter is delivered to the next `notified()` — no lost wakeup.
    pub async fn engaged_edge(&self) {
        self.notify.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_runtime::{deterministic::Runner, Runner as _};

    // The lag is the whole point of the pair, and it is written by TWO
    // independent tasks — so it must be recomputed by whichever wrote last, and
    // must not render the ordinary between-writes overshoot as a negative spike.
    #[test]
    fn plane_clock_lag_is_recomputed_by_either_writer_and_floored() {
        let clock = PlaneClock::default();
        clock.record_dkg_clock(1_000);
        assert_eq!(
            clock.snapshot(),
            (0, 1_000, 0),
            "DKG ahead of a silent ordering half"
        );

        clock.record_ordering_tip(1_006);
        assert_eq!(clock.snapshot(), (1_006, 1_000, 6));

        // The ordering writer moved last; the DKG writer catching up must clear
        // the lag without a further tip.
        clock.record_dkg_clock(1_006);
        assert_eq!(clock.snapshot(), (1_006, 1_006, 0));

        // Execution one block ahead of the tip this node has observed is the
        // ordinary interleaving, not a fault.
        clock.record_dkg_clock(1_007);
        assert_eq!(clock.snapshot().2, 0);
    }

    #[test]
    fn degrade_and_recover_round_trip() {
        let m = SyncMetrics::default();
        assert_eq!(m.degraded_value(SyncReason::NoPeers), 0);
        m.degrade(SyncReason::NoPeers);
        assert_eq!(m.degraded_value(SyncReason::NoPeers), 1);
        // Reasons are independent labels — degrading one leaves the others clear.
        assert_eq!(m.degraded_value(SyncReason::BoundaryHook), 0);
        m.recover(SyncReason::NoPeers);
        assert_eq!(m.degraded_value(SyncReason::NoPeers), 0);
    }

    #[test]
    fn safety_halt_latches_and_raises_its_reason_gauge() {
        let metrics = SyncMetrics::default();
        let halt = SafetyHalt::new(metrics.clone());
        assert!(!halt.is_engaged());
        assert_eq!(metrics.degraded_value(SyncReason::ResultDivergence), 0);

        halt.engage(SyncReason::ResultDivergence);
        assert!(halt.is_engaged(), "engage latches the halt");
        assert_eq!(
            metrics.degraded_value(SyncReason::ResultDivergence),
            1,
            "engage raises the shared reason gauge"
        );

        // The latch is permanent: a second engage (a different reason) keeps it
        // engaged and never clears — recovery is external, not in-node.
        halt.engage(SyncReason::L1Fork);
        assert!(halt.is_engaged());
        // ...and the FIRST verdict stays the recorded diagnosis; the second is
        // downstream of it.
        assert_eq!(halt.reason(), Some(SyncReason::ResultDivergence));
        assert_eq!(metrics.safety_halt_engaged.get(), 1);
    }

    fn scratch_marker(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fluent-safety-halt-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        dir.join("safety_halt")
    }

    // The defect this closes: the latch has no `disengage` by design, yet a
    // restart re-created it clear — a halted node came back a full signer with
    // no record of what it refused.
    #[test]
    fn a_restart_re_engages_from_the_datadir_marker_with_the_reason() {
        let marker = scratch_marker("restart");
        let first = SafetyHalt::restoring(SyncMetrics::default(), marker.clone());
        assert!(!first.is_engaged(), "a fresh datadir starts healthy");
        first.engage(SyncReason::ResultDivergence);

        // A new process, a new latch, the SAME datadir.
        let metrics = SyncMetrics::default();
        let restarted = SafetyHalt::restoring(metrics.clone(), marker.clone());
        assert!(restarted.is_engaged(), "the marker re-engages the latch");
        assert_eq!(restarted.reason(), Some(SyncReason::ResultDivergence));
        // Both the labeled reason and the unlabeled halt gauge are up BEFORE the
        // first consensus event — `register` ran on this same `SyncMetrics`.
        assert_eq!(metrics.degraded_value(SyncReason::ResultDivergence), 1);
        assert_eq!(metrics.safety_halt_engaged.get(), 1);

        // Operator clears it the only way there is: delete the file.
        std::fs::remove_file(&marker).expect("marker written");
        let cleared = SafetyHalt::restoring(SyncMetrics::default(), marker.clone());
        assert!(!cleared.is_engaged());
        assert_eq!(cleared.reason(), None);
        let _ = std::fs::remove_dir_all(marker.parent().expect("marker has a parent"));
    }

    // A marker written by another build names a reason this one cannot decode.
    // The FILE is the halt record, so it must still latch — losing the reason
    // must not lose the halt.
    #[test]
    fn an_undecodable_marker_still_comes_up_halted() {
        let marker = scratch_marker("undecodable");
        std::fs::create_dir_all(marker.parent().expect("marker has a parent"))
            .expect("scratch dir");
        std::fs::write(&marker, "some_future_reason\n").expect("write marker");

        let metrics = SyncMetrics::default();
        let halt = SafetyHalt::restoring(metrics.clone(), marker.clone());
        assert!(
            halt.is_engaged(),
            "an unknown reason must not clear the halt"
        );
        assert_eq!(halt.reason(), None);
        assert_eq!(
            metrics.safety_halt_engaged.get(),
            1,
            "the unlabeled gauge is what makes an undecodable halt visible"
        );
        let _ = std::fs::remove_dir_all(marker.parent().expect("marker has a parent"));
    }

    // A latch with no datadir (tests, in-process fixtures) must not write a
    // marker into whatever the process CWD happens to be.
    #[test]
    fn a_markerless_latch_persists_nothing() {
        let halt = SafetyHalt::new(SyncMetrics::default());
        halt.engage(SyncReason::ElInvalid);
        assert!(halt.is_engaged());
        assert_eq!(halt.reason(), Some(SyncReason::ElInvalid));
        assert_eq!(halt.marker_path(), None);
    }

    // The marker round-trips through the label, so a reason added without a
    // `from_label` arm cannot silently become an undecodable marker.
    #[test]
    fn every_sync_reason_label_round_trips() {
        for reason in [
            SyncReason::NoPeers,
            SyncReason::ActivationWait,
            SyncReason::AwaitingUpstream,
            SyncReason::AuthRotate,
            SyncReason::EngineRetry,
            SyncReason::LandingWait,
            SyncReason::CrashRecover,
            SyncReason::BoundaryHook,
            SyncReason::FinalizeApply,
            SyncReason::ResultDivergence,
            SyncReason::L1Fork,
            SyncReason::ElInvalid,
        ] {
            assert_eq!(
                SyncReason::from_label(reason.as_str()),
                Some(reason),
                "{} does not round-trip",
                reason.as_str()
            );
        }
        assert_eq!(SyncReason::from_label("not_a_reason"), None);
    }

    #[test]
    fn safety_halt_edge_fires_on_first_engage() {
        let runner = Runner::default();
        runner.start(|_ctx| async move {
            let halt = SafetyHalt::default();
            halt.engage(SyncReason::ElInvalid);
            // `notify_one` stored the permit before the waiter armed — the edge
            // still resolves (no lost wakeup), so this does not hang.
            halt.engaged_edge().await;
            assert!(halt.is_engaged());
        });
    }

    #[test]
    fn register_encodes_metric_names_and_reason_labels() {
        let runner = Runner::default();
        runner.start(|ctx| async move {
            let m = SyncMetrics::default();
            m.register(&ctx);
            m.degrade(SyncReason::LandingWait);
            m.engine_transient_retry.inc();
            let scrape = ctx.encode();
            assert!(
                scrape.contains("dpos_sync_degraded"),
                "gauge family registered: {scrape}"
            );
            assert!(
                scrape.contains("reason=\"landing_wait\""),
                "snake_case reason label present: {scrape}"
            );
            assert!(
                scrape.contains("engine_transient_retry_total"),
                "counter registered: {scrape}"
            );
            assert!(
                scrape.contains("crash_recover_gap_blocks"),
                "gap gauge registered: {scrape}"
            );
            assert!(
                scrape.contains("el_sync_stalled_with_peers_total"),
                "connected-but-wedged counter registered: {scrape}"
            );
            assert!(
                scrape.contains("dpos_safety_halt_engaged"),
                "halt latch gauge registered: {scrape}"
            );
        });
    }
}

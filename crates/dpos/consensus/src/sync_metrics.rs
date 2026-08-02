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
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;

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
/// Arc-backed → cheap to clone; every clone shares one latch + one gauge family.
#[derive(Clone, Default)]
pub struct SafetyHalt {
    engaged: Arc<AtomicBool>,
    notify: Arc<Notify>,
    metrics: SyncMetrics,
}

impl SafetyHalt {
    /// Build a latch that raises its `reason` gauge on the SHARED (already
    /// registered) [`SyncMetrics`] from `dpos.rs::launch`.
    pub fn new(metrics: SyncMetrics) -> Self {
        Self {
            engaged: Arc::default(),
            notify: Arc::default(),
            metrics,
        }
    }

    /// Latch the halt + raise `dpos_sync_degraded{reason}=1`. Idempotent; the
    /// epoch-manager wakeup fires once, on the 0→1 edge.
    pub fn engage(&self, reason: SyncReason) {
        self.metrics.degrade(reason);
        if !self.engaged.swap(true, Ordering::SeqCst) {
            self.notify.notify_one();
        }
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
        });
    }
}

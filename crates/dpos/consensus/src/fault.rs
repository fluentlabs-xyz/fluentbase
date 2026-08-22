//! The executor fault taxonomy (family 5): ONE closed classification of every
//! fallible executor/boundary operation, replacing the per-site behavioral
//! classifiers that each new soak fix used to add.
//!
//! The rule the taxonomy encodes is the fork-safety split that N sites used to
//! enforce by convention (a comment at each call site): **retry ⇔ transport
//! `Err`/`Syncing`; SafetyHalt ⇔ `Ok(Invalid)`**. Here it is a *type-level*
//! fact — the layer that OWNS a concrete error maps it into a [`FaultClass`],
//! and the executor routes on the class. The dispositions themselves stay
//! implemented by the executor's existing mechanisms (the bounded/convergent
//! derive belts, degrade-retry, non-blocking defer, `SafetyHalt` engage,
//! abort-all) — the taxonomy is the shared VOCABULARY the leaf mappers speak,
//! not a second dispatch layer over those mechanisms.
//!
//! Classification happens exactly where the concrete error type exists (the
//! node-side deriver/importer/staking-reader — see [`EngineError`],
//! `crate::application::BeaconEngineLike`, `fluentbase-node`'s
//! `classify_derive_fault`, `cert_inlet::committee_read_fault`), and is carried
//! as a typed verdict, never as a display string re-parsed in consensus code.

use crate::sync_metrics::SyncReason;

/// Why a work item is deferred: skipped non-blockingly and re-presented later by
/// the pipeline (never a retry loop that blocks the draining task — the
/// no-sleep rule is a property of the class, not of the site).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferReason {
    /// `committee[E]` is not yet committed at the finalized tip — boundary lag
    /// while the executor catches up (cert-inlet, first-of-epoch cert).
    CommitteeNotCommitted,
    /// reth transiently dropped executed state at/below the finalized hash
    /// during pipeline backfill (staking-reader `StateNotMaterialized`).
    StateNotMaterialized,
    /// Guard #2: the node is ≥ K behind but the committee-attested body at
    /// `h + K` is not backfilled yet, so the convergence check cannot run —
    /// park the finalized block + re-poke event-driven (`DeriveOutcome`).
    NeedAttestation,
    /// A SPECULATIVE (notarization-path) derive failed. The finalized path is
    /// the sole authority and derives this height from the child witness
    /// regardless, so the speculative work item is skipped, not retried here.
    SpecDeriveFailed,
    /// reth answered a SPECULATIVE forkchoice update with a non-`Valid`,
    /// non-`Syncing` verdict — see the executor's `spec_execute` for why that
    /// verdict is NOT a fork-safety witness on a notarized-but-unfinalized
    /// block (the finalized path re-renders it if the branch commits).
    SpecFcuRejected,
}

impl DeferReason {
    /// snake_case label (metric/dashboard series continuity).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommitteeNotCommitted => "committee_not_committed",
            Self::StateNotMaterialized => "state_not_materialized",
            Self::NeedAttestation => "need_attestation",
            Self::SpecDeriveFailed => "spec_derive_failed",
            Self::SpecFcuRejected => "spec_fcu_rejected",
        }
    }
}

/// The closed executor fault taxonomy. Every fallible operation the executor
/// routes maps to exactly one of these; the mapping lives at the layer that
/// owns the concrete error, and the routing site matches the class directly:
///
/// - [`Self::TransientBounded`]/[`Self::TransientConvergent`] → the derive
///   belt's two budget loops (exhaustion is loud, propagating the last error);
/// - [`Self::TransientExternal`]`(reason)` → degrade-visible + retry-forever /
///   defer to reconvergence (Decision A: never actor-death on a correlated
///   cause);
/// - [`Self::Defer`]`(reason)` → skip the work item non-blockingly + reason
///   counter;
/// - [`Self::ForkSafety`]`(reason)` → `SafetyHalt::engage(reason)` + `Err` →
///   `park_halted`;
/// - [`Self::Corruption`] → `Err` with the latch NOT engaged → supervisor
///   abort-all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultClass {
    /// Bounded retry; the trigger ALSO fires on genuine corruption (torn
    /// static-file reads, visibility lag), so exhaustion is loud and propagates
    /// the original error. The two-phase fast/slow budget lives in
    /// `fluentbase-node`'s `derive.rs` next to the reth error types it parses.
    TransientBounded,
    /// Unbounded-ish retry that provably makes forward progress per attempt
    /// (mdbx read-txn timeout — the changeset cache warms per block; 64
    /// attempts, no backoff, budget declared in `derive.rs`).
    TransientConvergent,
    /// Retry forever, degraded-visible; the cause is external/correlated
    /// (engine transport, EL apply lag) so NEVER actor-death (Decision A).
    TransientExternal(SyncReason),
    /// Skip this work item non-blockingly; the pipeline re-presents it.
    Defer(DeferReason),
    /// Local derivation would extend a chain honest peers reject — latch the
    /// `SafetyHalt` (result divergence, EL `Invalid`, L1 fork).
    ForkSafety(SyncReason),
    /// Idiosyncratic local corruption/misconfig (hole-below-floor, unfillable
    /// marshal gap, non-deterministic re-derive); loud actor death is correct.
    Corruption,
}

impl FaultClass {
    /// snake_case class label for the router's `class=` metric dimension.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TransientBounded => "transient_bounded",
            Self::TransientConvergent => "transient_convergent",
            Self::TransientExternal(_) => "transient_external",
            Self::Defer(_) => "defer",
            Self::ForkSafety(_) => "fork_safety",
            Self::Corruption => "corruption",
        }
    }

    /// The class's payload label for the router's `reason=` metric dimension;
    /// `"none"` for the three classes that carry no payload (a bounded label
    /// set — never a formatted error string).
    pub const fn reason_str(self) -> &'static str {
        match self {
            Self::TransientExternal(reason) | Self::ForkSafety(reason) => reason.as_str(),
            Self::Defer(reason) => reason.as_str(),
            Self::TransientBounded | Self::TransientConvergent | Self::Corruption => "none",
        }
    }
}

/// A [`FaultClass`] verdict carried together with the concrete cause that
/// produced it — the type every fallible executor boundary returns.
///
/// The point is what it makes IMPOSSIBLE. An `eyre::Report` lets a routing site
/// reduce any failure to a `warn!`; a `Fault` forces that site to match on
/// [`Self::class`], so a [`FaultClass::ForkSafety`] cannot be logged away. The
/// executor's ONE disposition router (`Actor::dispatch_fault`) is the only place
/// that turns a class into an action — and it is therefore the only place that
/// engages the `SafetyHalt` latch, so "engage here and hope the `Err` is
/// propagated" is no longer expressible.
#[derive(Debug)]
pub struct Fault {
    class: FaultClass,
    cause: eyre::Report,
}

impl Fault {
    /// Pair an already-decided class with its cause.
    pub const fn new(class: FaultClass, cause: eyre::Report) -> Self {
        Self { class, cause }
    }

    /// Local derivation would extend a chain honest peers reject: the router
    /// engages `SafetyHalt::engage(reason)` and parks. The latch is NOT touched
    /// here — building a `Fault` has no side effects.
    pub const fn fork_safety(reason: SyncReason, cause: eyre::Report) -> Self {
        Self::new(FaultClass::ForkSafety(reason), cause)
    }

    /// Idiosyncratic local corruption/misconfig: loud actor death, latch clear.
    pub const fn corruption(cause: eyre::Report) -> Self {
        Self::new(FaultClass::Corruption, cause)
    }

    /// Skip this work item non-blockingly; the pipeline re-presents it.
    pub const fn defer(reason: DeferReason, cause: eyre::Report) -> Self {
        Self::new(FaultClass::Defer(reason), cause)
    }

    /// External/correlated cause (engine transport, EL apply lag): degrade
    /// visibly and continue — never actor-death (Decision A).
    pub const fn transient_external(reason: SyncReason, cause: eyre::Report) -> Self {
        Self::new(FaultClass::TransientExternal(reason), cause)
    }

    pub const fn class(&self) -> FaultClass {
        self.class
    }

    /// The cause, for typed downcasts at a routing site that recognises a
    /// specific leaf error (the executor's parent-visibility park).
    pub const fn cause(&self) -> &eyre::Report {
        &self.cause
    }

    pub fn into_parts(self) -> (FaultClass, eyre::Report) {
        (self.class, self.cause)
    }
}

/// An UNCLASSIFIED error crossing a classified boundary is
/// [`FaultClass::Corruption`] — the LOUD disposition, so an unmapped leaf can
/// only ever be too noisy, never silently swallowed. This is also exactly what
/// an untyped `eyre::Report` already did at the executor's fatal sites (log +
/// actor death + supervisor abort-all), so `?` on a plain eyre error keeps its
/// pre-taxonomy behaviour. A boundary whose failures must stay best-effort (the
/// speculative path) therefore has to classify EXPLICITLY.
impl From<eyre::Report> for Fault {
    fn from(cause: eyre::Report) -> Self {
        Self::corruption(cause)
    }
}

/// A failure at the reth engine boundary that produced NO verdict — as opposed
/// to a semantic `Ok(Invalid)` verdict, which rides in the `Ok` half. Distinct
/// type so the no-verdict-vs-verdict split is TYPE-LEVEL, not a convention at
/// each call site: an [`crate::application::BeaconEngineLike`] method returns
/// the verdict in `Ok(..)` and the no-verdict failure in `Err(EngineError)`.
///
/// It carries its own [`FaultClass`] because "the request did not produce a
/// verdict" has two structurally different causes with OPPOSITE dispositions,
/// and collapsing them is how a permanent local condition ends up in a
/// retry-forever loop:
///
/// - [`Self::transport`] — reth never processed the request (closed engine/tree
///   channel, dropped oneshot, internal reth error): honestly transient,
///   [`FaultClass::TransientExternal`]`(EngineRetry)`, retried forever.
/// - [`Self::anchor_inconsistent`] — reth PROCESSED the forkchoice update and
///   rejected the state this node named: [`FaultClass::Corruption`].
#[derive(Debug)]
pub struct EngineError {
    class: FaultClass,
    message: String,
}

impl EngineError {
    /// The request never reached a verdict (a closed channel / an RPC-handle
    /// blip). The node-side importer stringifies its concrete engine-handle
    /// error here — the ONE place the display is captured, next to the type it
    /// parses.
    pub fn transport(source: impl std::fmt::Display) -> Self {
        Self {
            class: FaultClass::TransientExternal(SyncReason::EngineRetry),
            message: source.to_string(),
        }
    }

    /// reth rejected the forkchoice STATE itself — it cannot resolve the
    /// finalized/safe hash this node named in its own canonical chain. Retrying
    /// re-sends the same unresolvable hashes forever, so this is
    /// [`FaultClass::Corruption`]: the node's own anchor is inconsistent with
    /// its own EL, which says nothing about what the network believes (so it is
    /// NOT [`FaultClass::ForkSafety`]) and cannot heal on its own.
    pub fn anchor_inconsistent(source: impl std::fmt::Display) -> Self {
        Self {
            class: FaultClass::Corruption,
            message: source.to_string(),
        }
    }

    /// The taxonomy verdict this engine failure carries.
    pub const fn fault_class(&self) -> FaultClass {
        self.class
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    // A no-verdict engine failure is transient ONLY when reth never processed
    // the request. `InvalidState` (reth processed it and rejected the anchor we
    // named) used to share this class and was therefore retried forever.
    #[test]
    fn engine_transport_retries_but_an_inconsistent_anchor_is_corruption() {
        let e = EngineError::transport("engine tree channel closed");
        assert_eq!(
            e.fault_class(),
            FaultClass::TransientExternal(SyncReason::EngineRetry)
        );
        // Displayable + std::error::Error so it `?`-converts into eyre chains at
        // callers that don't distinguish transport (they just propagate).
        assert_eq!(e.to_string(), "engine tree channel closed");
        let _dyn: &dyn std::error::Error = &e;

        let anchor = EngineError::anchor_inconsistent("invalid forkchoice state");
        assert_eq!(
            anchor.fault_class(),
            FaultClass::Corruption,
            "a rejected forkchoice STATE must never land in a retry-forever class"
        );
    }

    // The blanket conversion is the loud disposition, not the quiet one: an
    // unmapped leaf error must be able to kill the actor, never to be dropped.
    #[test]
    fn an_unclassified_eyre_error_converts_to_corruption() {
        let fault: Fault = eyre::eyre!("something nobody mapped").into();
        assert_eq!(fault.class(), FaultClass::Corruption);
        assert_eq!(fault.cause().to_string(), "something nobody mapped");
    }

    // The router's metric dimensions are a BOUNDED label set — every class
    // renders a fixed pair, never a formatted error string.
    #[test]
    fn fault_class_labels_are_bounded_and_carry_the_payload_reason() {
        assert_eq!(FaultClass::Corruption.as_str(), "corruption");
        assert_eq!(FaultClass::Corruption.reason_str(), "none");
        let fs = FaultClass::ForkSafety(SyncReason::ElInvalid);
        assert_eq!(fs.as_str(), "fork_safety");
        assert_eq!(fs.reason_str(), "el_invalid");
        let defer = FaultClass::Defer(DeferReason::SpecFcuRejected);
        assert_eq!(defer.as_str(), "defer");
        assert_eq!(defer.reason_str(), "spec_fcu_rejected");
    }

    // The defer labels are the metric series names — pinned for continuity with
    // the pre-taxonomy `dpos_cert_inlet_committee_read_deferred_total` labels.
    #[test]
    fn defer_reason_labels_match_the_metric_series() {
        assert_eq!(
            DeferReason::CommitteeNotCommitted.as_str(),
            "committee_not_committed"
        );
        assert_eq!(
            DeferReason::StateNotMaterialized.as_str(),
            "state_not_materialized"
        );
        assert_eq!(DeferReason::NeedAttestation.as_str(), "need_attestation");
        assert_eq!(DeferReason::SpecDeriveFailed.as_str(), "spec_derive_failed");
        assert_eq!(DeferReason::SpecFcuRejected.as_str(), "spec_fcu_rejected");
    }
}

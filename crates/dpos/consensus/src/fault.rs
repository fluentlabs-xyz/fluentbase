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
//! node-side deriver/importer/staking-reader — see [`TransportError`],
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
}

impl DeferReason {
    /// snake_case label (metric/dashboard series continuity).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommitteeNotCommitted => "committee_not_committed",
            Self::StateNotMaterialized => "state_not_materialized",
            Self::NeedAttestation => "need_attestation",
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

/// A transport-layer failure at the reth engine boundary (a closed engine
/// channel / an RPC-handle blip) — as opposed to a semantic `Ok(Invalid)`
/// verdict. Distinct type so the transport-vs-verdict split is TYPE-LEVEL, not
/// a convention at each call site: an [`crate::application::BeaconEngineLike`]
/// method returns the verdict in `Ok(..)` and transport in `Err(TransportError)`,
/// so a verdict can never be folded into the transport error and both engine
/// entry points (FCU + import) share the same transport class.
///
/// Classified [`FaultClass::TransientExternal`]`(EngineRetry)`: retried forever /
/// deferred to reconvergence, degraded-visible, NEVER actor-death (Decision A).
#[derive(Debug)]
pub struct TransportError(pub String);

impl TransportError {
    /// Build from anything displayable (the node-side importer stringifies its
    /// concrete engine-handle / channel error here — the ONE place the display
    /// is captured, next to the type it parses).
    pub fn new(source: impl std::fmt::Display) -> Self {
        Self(source.to_string())
    }

    /// The taxonomy verdict for a transport error — always the same class, which
    /// is the whole point of the typed split.
    pub const fn fault_class(&self) -> FaultClass {
        FaultClass::TransientExternal(SyncReason::EngineRetry)
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportError {}

#[cfg(test)]
mod tests {
    use super::*;

    // A transport error is ALWAYS the same external class — the type-level
    // guarantee the whole split rests on.
    #[test]
    fn transport_error_is_always_transient_external_engine_retry() {
        let e = TransportError::new("engine tree channel closed");
        assert_eq!(
            e.fault_class(),
            FaultClass::TransientExternal(SyncReason::EngineRetry)
        );
        // Displayable + std::error::Error so it `?`-converts into eyre chains at
        // callers that don't distinguish transport (they just propagate).
        assert_eq!(e.to_string(), "engine tree channel closed");
        let _dyn: &dyn std::error::Error = &e;
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
    }
}

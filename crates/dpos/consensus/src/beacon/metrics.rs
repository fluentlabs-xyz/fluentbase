//! Beacon observability counters, registered on the commonware metrics registry
//! (scraped at `:19100` via `Metrics::register`, NOT the `metrics::` macro recorder
//! which lands on reth's registry and is invisible there).
//!
//! A single [`BeaconMetrics`] is created + registered once in `dpos.rs::launch`
//! (against the launch context) and cloned into the executor (`seed_active` /
//! `digest_fallback`), the DKG actor (`dkg_ceremony_ok` / `dkg_ceremony_fail`), and
//! each per-epoch engine (the demote counters). Each metric is `Arc`-backed, so the
//! struct is cheap to clone and every clone shares one counter.

use commonware_runtime::Metrics;
use prometheus_client::metrics::counter::Counter;

/// Beacon counters. See the module docs for the registration + clone topology.
#[derive(Clone, Debug, Default)]
pub struct BeaconMetrics {
    /// A block's `prev_randao` was the verified threshold seed (`assurance=true`).
    pub seed_active: Counter,
    /// A beacon-active block fell back to `order.digest()` (seed absent or failed
    /// σ-verify vs `PK_E`). The Stage-2 certify hook Nullifies a beacon-active
    /// boundary before it finalizes, so this counts the LOCAL pre-Nullify observation
    /// on a node that derived ahead of the Nullify; smoke D1 asserts it is 0
    /// post-anchor on a healthy chain.
    pub digest_fallback: Counter,
    /// A live-DKG ceremony finalized (`PK_E` + share computed + stored).
    pub dkg_ceremony_ok: Counter,
    /// A live-DKG ceremony failed to finalize after the ready-probe (epoch beacon
    /// stalls until a reshare / next ceremony).
    pub dkg_ceremony_fail: Counter,
    /// A per-epoch engine self-demoted to the cert-follow plane because it holds no
    /// local beacon polynomial for the epoch (`NoBeaconPolynomial`).
    pub engine_demoted_no_polynomial: Counter,
    /// A per-epoch engine self-demoted because the operator's validator was rotated
    /// out of the epoch's committee (`RotatedOut`).
    pub engine_demoted_rotated_out: Counter,
}

impl BeaconMetrics {
    /// Register every counter on the commonware registry. Call once, against the
    /// launch context (mirrors `executor.rs`'s `pending_finalizations` gauge).
    pub fn register(&self, ctx: &impl Metrics) {
        ctx.register(
            "beacon_seed_active_total",
            "Blocks whose prev_randao was the verified threshold seed.",
            self.seed_active.clone(),
        );
        ctx.register(
            "beacon_digest_fallback_total",
            "Beacon-active blocks that fell back to order.digest() (seed absent/unverified). \
             0 post-anchor on a healthy chain.",
            self.digest_fallback.clone(),
        );
        ctx.register(
            "dkg_ceremony_ok_total",
            "Live-DKG ceremonies that finalized (PK_E + share stored).",
            self.dkg_ceremony_ok.clone(),
        );
        ctx.register(
            "dkg_ceremony_fail_total",
            "Live-DKG ceremonies that failed to finalize after the ready-probe.",
            self.dkg_ceremony_fail.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_no_polynomial_total",
            "Per-epoch engines self-demoted to cert-follow for lack of a local beacon polynomial.",
            self.engine_demoted_no_polynomial.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_rotated_out_total",
            "Per-epoch engines self-demoted because the validator was rotated out of the committee.",
            self.engine_demoted_rotated_out.clone(),
        );
    }
}

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
    /// A would-be signer was demoted to verify-only by the promote VALUE-gate:
    /// its locally-resolved `PK_epoch` differs from the network-attested key
    /// (W4 observed-outcome entry / the finalized boundary block's own
    /// `beacon_outcome`). Non-zero = a diverged local key reconstruction was
    /// caught before it could sign/publish (soak 2026-07-14 class).
    pub engine_demoted_key_divergence: Counter,
    /// A would-be signer was demoted to verify-only by the promote SHARE-gate: its
    /// resolved DKG share does not verify against its own sharing. Distinct from
    /// the VALUE gate above, which compares the GROUP key against the network and
    /// is a no-op before the network has attested one. Load-bearing since every
    /// Nullify carries a seed partial: with `t == quorum`, one such member on the
    /// plane makes the nullify quorum unreachable during a stall, when there are no
    /// proposals to expose the bad share on the notarize path.
    pub engine_demoted_bad_share: Counter,
    /// A per-epoch engine spawn was deferred because the marshal does not hold the
    /// previous epoch's terminal block — the `Inline::genesis(E)` precondition. The
    /// member registers verify-only meanwhile: no proposals, no votes. Normally
    /// transient (the block is still being backfilled and the next derived block
    /// re-pokes the reconciler); persistently non-zero across an epoch means the
    /// height sits below the marshal floor, where no repair path fetches it, and the
    /// boundary seeding at the floor-raise sites either did not run or failed.
    pub engine_spawn_deferred: Counter,
    /// A consensus-pinned dealer-log index named a position outside the committed
    /// committee, and the ceremony skipped it. Nothing can ever satisfy such an entry
    /// (the resolver fetches per-DEALER), so before the skip it held `all_held=false`
    /// forever and wedged the epoch's DKG in silence. With the `order_block` codec
    /// bound in place an honest chain cannot produce one: non-zero means either a
    /// Byzantine proposer got a block past an accept-biased vote gate, or the
    /// committee the ceremony reads disagrees with the one the logs were numbered
    /// against — the 2026-07-21 idx-stall class. 0 on a healthy chain.
    pub dkg_pinned_idx_out_of_range: Counter,
    /// A ceremony reached its settle deadline without qualifying and deferred to the
    /// epoch boundary. Previously this wait was completely silent, so any stall of
    /// this family could only be diagnosed post-mortem from a wedged boundary.
    /// Counted once per epoch per reason.
    pub dkg_finalize_deferred: Counter,
    /// A live-epoch engine handle was found COMPLETED at a reconcile edge while
    /// its epoch is still the live frontier, and the manager re-ran the spawn
    /// path. Under `catch_panics(true)` a child engine panic completes the
    /// `Handle` without reaching the manager, so nothing else respawns it —
    /// non-zero means such a dead engine was detected and revived (or the engine
    /// exited early for any other reason). 0 on a healthy chain.
    pub engine_respawned: Counter,
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
        ctx.register(
            "epoch_engine_demoted_key_divergence_total",
            "Would-be signers demoted to verify-only because the locally-resolved PK_epoch \
             differs from the network-attested key.",
            self.engine_demoted_key_divergence.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_bad_share_total",
            "Would-be signers demoted to verify-only because the resolved DKG share does not \
             verify against its own sharing.",
            self.engine_demoted_bad_share.clone(),
        );
        ctx.register(
            "epoch_engine_spawn_deferred_total",
            "Per-epoch engine spawns deferred for a missing E-1 boundary block (member is \
             verify-only meanwhile). Persistently non-zero = the height is below the marshal \
             floor and boundary seeding did not cover it.",
            self.engine_spawn_deferred.clone(),
        );
        ctx.register(
            "dpos_dkg_pinned_idx_out_of_range_total",
            "Consensus-pinned dealer-log indices with no position in the committed committee, \
             skipped by the ceremony. 0 on a healthy chain.",
            self.dkg_pinned_idx_out_of_range.clone(),
        );
        ctx.register(
            "dpos_dkg_finalize_deferred_total",
            "Ceremonies that reached their settle deadline unqualified and deferred to the \
             epoch boundary (once per epoch per reason).",
            self.dkg_finalize_deferred.clone(),
        );
        ctx.register(
            "epoch_engine_respawned_total",
            "Live-frontier engines found completed at a reconcile edge (panic caught by \
             catch_panics, or early exit) and revived via the spawn path.",
            self.engine_respawned.clone(),
        );
    }
}

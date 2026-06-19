//! Per-epoch threshold randomness beacon: a separate BLS12-381 threshold key
//! (consensus stays multisig for attributability) that committee[E] deals to
//! ITSELF (Model B: dealers == players) via a commonware Joint-Feldman self-DKG
//! run during epoch E-1 (see [`actor`]/[`ceremony`]).
//!
//! The per-round seed is the recovered threshold signature over
//! `(seed_namespace ‖ round)`. It rides INSIDE the combined consensus scheme:
//! the seed partial is carried in each Notarize/Finalize vote and recovered
//! from the round's certificate (notarization OR finalization), so it is
//! available at NOTARIZATION (where speculative execution consumes it), not only
//! after finalization. `prev_randao(R) = H(seed(R))` for the block ordered at
//! round R (see [`seed::prev_randao_for_round`]). The seed is unique by
//! construction (any ≥t partials recover the same value) and unpredictable to
//! the txs of block R (txs are assembled before the round's quorum exists).
//!
//! KNOWN LIMITATION — abort-to-next-view re-roll grind (DEFERRED): because
//! `seed(R)` is consumed for block R's `prev_randao` and is
//! computable at notarization of round R, a Byzantine leader who dislikes the
//! resulting `prev_randao(R)` can force a Nullify (plain timeout/skip — NOT
//! slashable) to move the block to round R+1, drawing a fresh `seed(R+1)` and
//! thus a fresh `prev_randao`. Each such abort costs only a view change, so the
//! randomness for height R is grindable across re-rolls. The standard fix is a
//! K-LAG on seed CONSUMPTION: have block R consume `seed(R−k)` (k aligned with
//! the existing K=3 result-final lag in `order_block`), so by the time block R's
//! content is fixed, `seed(R−k)` was finalized k rounds earlier and cannot be
//! re-rolled without reverting finalized history (info-theoretically unbiasable
//! with the delay — uniqueness + Lagrange<t). This is a multi-repo protocol
//! change (deriver seed selection, executor seed lookup keyed off R−k, the STF
//! zkVM mirror `verify_block_prev_randao`, the boundary-edge / catch-up
//! seed-availability machinery, on-chain commit binding, and the first-K-blocks
//! and epoch-boundary edge cases) and is tracked separately — do NOT bolt on a
//! partial K-lag here, it must be symmetric across the node and the STF guest.
//!
//! This module holds the wire [`types`] shared with `order_block` and the
//! [`seed`] cryptographic primitives; the DKG ceremony and the p2p sub-protocol
//! actors live in [`actor`] / [`ceremony`] / [`dkg`].

pub mod actor;
pub mod ceremony;
pub mod certify;
pub mod dkg;
pub mod dkg_msg;
pub mod metrics;
pub mod outcome;
pub mod seed;
pub mod share_state;
pub mod types;
pub mod wire;

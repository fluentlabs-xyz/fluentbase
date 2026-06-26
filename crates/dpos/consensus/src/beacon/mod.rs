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
//! round R (see [`seed::prev_randao_from_seed`]). The seed is unique by
//! construction (any ≥t partials recover the same value) and unpredictable to
//! the txs of block R (txs are assembled before the round's quorum exists).
//!
//! This module holds the per-height `Seed` wire type and the [`seed`]
//! cryptographic primitives; the DKG ceremony and the p2p sub-protocol
//! actors live in [`actor`] / [`ceremony`].

pub mod actor;
pub mod ceremony;
pub mod certify;
/// Local single-process DKG oracle — used only by tests across the crate (the
/// production path is the networked [`actor`]/[`ceremony`]). `#[cfg(test)]`-gated
/// so it is not compiled into release builds.
#[cfg(test)]
pub mod dkg_oracle;
pub mod dkg_msg;
pub mod log_resolver;
pub mod metrics;
pub mod outcome;
pub mod seed;
pub mod share_state;
pub mod wire;

//! Per-epoch threshold randomness beacon: a separate BLS12-381 threshold key
//! (consensus stays multisig for attributability) that the committee active in
//! epoch E deals fresh to committee[E+1] via commonware Joint-Feldman DKG, and
//! that signs a per-height seed AFTER each height finalizes. `prev_randao(h) =
//! H(seed(h))` — unbiasable (sign-after-finalize) and unpredictable to the txs
//! of block h (frozen before seed(h) exists).
//!
//! This module holds the wire [`types`] shared with `order_block` and the
//! [`seed`] cryptographic primitives; the DKG ceremony and the p2p sub-protocol
//! actors are added in subsequent increments.

pub mod ceremony;
pub mod dkg;
pub mod dkg_msg;
pub mod outcome;
pub mod seed;
pub mod types;
pub mod wire;

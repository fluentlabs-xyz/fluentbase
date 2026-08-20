//! Per-epoch threshold randomness beacon: a separate BLS12-381 threshold key
//! (consensus stays multisig for attributability) that committee[E] deals to
//! ITSELF (Model B: dealers == players) via a commonware Joint-Feldman self-DKG
//! run during epoch E-1.
//!
//! The per-round seed is the recovered threshold signature over
//! `(seed_namespace ‖ round)`. It rides INSIDE the combined consensus scheme:
//! the seed partial is carried in each Notarize/Finalize vote and recovered
//! from the round's certificate (notarization OR finalization), so it is
//! available at NOTARIZATION (where speculative execution consumes it), not only
//! after finalization. `prev_randao(R) = H(seed(R))` for the block ordered at
//! round R (see [`prev_randao_from_seed`]). The seed is unique by construction
//! (any ≥t partials recover the same value) and unpredictable to the txs of
//! block R (txs are assembled before the round's quorum exists).
//!
//! # The boundary
//!
//! This module is CLOSED: every submodule is `pub(crate)` and what leaves it is
//! exactly what is re-exported here — the [`Seed`] wire type with its
//! `prev_randao` derivation, the two opaque key handles ([`BeaconKeys`],
//! [`AgreedKeys`]), and the plane facade ([`build`] and its config/result). How
//! the epoch key is agreed, where the artifact is stored, how a share is derived
//! and how a peer is served are all internal, and nothing above the beacon
//! assembles them.

pub(crate) mod actor;
pub(crate) mod artifact;
pub(crate) mod carry;
pub(crate) mod ceremony;
pub(crate) mod certify;
pub(crate) mod dkg_agree;
pub(crate) mod dkg_engine;
pub(crate) mod dkg_msg;
/// Local single-process DKG oracle — used only by the beacon's own tests (the
/// production path is the networked `actor`/`ceremony`). `#[cfg(test)]`-gated so
/// it is not compiled into release builds.
#[cfg(test)]
pub(crate) mod dkg_oracle;
pub(crate) mod dkg_transport;
pub(crate) mod key_journal;
pub(crate) mod keys;
pub(crate) mod log_resolver;
pub(crate) mod metrics;
pub(crate) mod outcome;
pub(crate) mod plane;
pub(crate) mod seed;
pub(crate) mod seed_journal;
pub(crate) mod share_state;
pub(crate) mod wire;

pub use keys::{AgreedKeys, BeaconKeys};
pub use plane::{build, Beacon, BeaconConfig, BeaconShared, BeaconWriteBack};
pub use seed::{prev_randao_from_seed, Seed};

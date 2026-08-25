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
//! Every submodule is `pub(crate)`, so the compiler enforces only half of this
//! boundary: nothing OUTSIDE `fluentbase-consensus` can name a submodule, while a
//! sibling file inside the crate still can. The other half is the rule that
//! production code reaches the beacon only through the two re-export tiers below
//! — so this list, not a grep for `beacon::`, is the module's front door.
//!
//! `pub use` — what leaves the crate: the [`Seed`] wire type with its
//! `prev_randao` derivation, the two opaque key handles ([`BeaconKeys`],
//! [`AgreedKeys`]), the plane facade ([`build`] and its config/result) and the
//! follower facade ([`for_follower`] and its config/result).
//!
//! `pub(crate) use` — the front door for the rest of this crate, and no wider:
//! the four items production code elsewhere in `fluentbase-consensus` genuinely
//! needs ([`absent_unregistered`], [`frozen_dkg_qual`], [`CommitteeSource`],
//! [`agreement_partition`]) but that no consumer of the crate should see.
//!
//! On neither list, and deliberately: how the epoch key is agreed, where the
//! artifact is stored, how a share is derived and how a peer is served. Nothing
//! above the beacon assembles them. The crate's own TESTS are the one standing
//! exception — they reach submodule paths directly (`beacon::keys::…`,
//! `beacon::surface::PlaneRandomness`, `beacon::surface::testing::…`) to build
//! fixtures out of the real rungs, and widening the front door for them would put
//! those internals in reach of production code too.
//!
//! The follower facade exists so that closing FLU-1167 did not have to open the
//! module: a follower needs `verify_artifact_for_epoch` and the artifact types,
//! both `pub(crate)`, so its provider is built INSIDE and the node hands in
//! capabilities (a fetch closure, a committee source, a `DkgQualFor`) exactly as
//! it already does for the plane.

pub(crate) mod actor;
pub(crate) mod artifact;
pub(crate) mod carry;
pub(crate) mod ceremony;
pub(crate) mod certify;
pub(crate) mod confirmations;
pub(crate) mod dkg_agree;
pub(crate) mod dkg_engine;
pub(crate) mod dkg_msg;
/// Local single-process DKG oracle — used only by the beacon's own tests (the
/// production path is the networked `actor`/`ceremony`). `#[cfg(test)]`-gated so
/// it is not compiled into release builds.
#[cfg(test)]
pub(crate) mod dkg_oracle;
pub(crate) mod dkg_transport;
pub(crate) mod follower;
pub(crate) mod key_journal;
pub(crate) mod keys;
pub(crate) mod log_resolver;
pub(crate) mod log_store;
pub(crate) mod metrics;
pub(crate) mod oracle;
pub(crate) mod outcome;
pub(crate) mod plane;
pub(crate) mod resolve;
pub(crate) mod seed;
pub(crate) mod seed_journal;
pub(crate) mod seed_resolver;
pub(crate) mod share_state;
pub(crate) mod surface;
pub(crate) mod verified_seed;
pub(crate) mod wire;

// The one retention window every per-epoch map in this module ages out on. It
// lives HERE, not in the module that happens to sweep on it: `share_state` needs
// the same number for its on-disk reconcile, and reaching sideways into `actor`
// for it made a policy constant look like an actor detail.
/// Trailing epochs past its own boundary for which a finalized/stalled epoch's DKG
/// journal (own `ReceivedDealing` views AND the shared QUAL logs) + the dealer-log
/// serve cache ([`log_store::DealerLogStore`])
/// are RETAINED — the recompute-heal window (§8.11.1). A demoted `committee[E]`
/// member (or a peer it serves) recomputes E's share from these while E is still
/// committee-relevant, instead of being swept the instant `now == E` and lingering a
/// verify-only observer until the next committee change.
///
/// Default `1` = "current epoch + 1 trailing". This is the BELTED default, NOT
/// "already sufficient": for the warm-member trigger (already caught up, mesh flapped)
/// the heal completes within 1 window; for the cold-sync refill/promote triggers the
/// heal DEPENDS on catch-up (EL sync + mesh reconnect + log refetch) finishing before
/// the target epoch's journal ages out of this window — those are ALSO fronted by the
/// harness warm-gate. Derivation for a wider window:
/// `JOURNAL_RETENTION_EPOCHS ≥ ceil(worst_case_catchup_seconds / epoch_seconds) + 1`,
/// `epoch_seconds ≈ EPOCH_INTERVAL × 1 s` (1 blk/s). Operational monitor:
/// `epoch_engine_demoted_no_polynomial` persisting > `JOURNAL_RETENTION_EPOCHS` epochs
/// for one identity = a demote whose logs aged out before catch-up → widen the window.
/// Size cost ≈ window × (~430 KiB QUAL set at n=51 + the per-dealer secret view bodies)
/// per retained epoch. Under-retention is SAFE: a member that finds the logs evicted
/// simply keeps fetching / stays a verify-only observer — it never adopts a wrong share
/// (the recompute self-check gates that), so widening only trades disk for heal reach.
pub(crate) const JOURNAL_RETENTION_EPOCHS: u64 = 1;

pub use actor::CommitteePairFor;
pub use follower::{for_follower, ArtifactFetch, FollowerBeacon, FollowerRandomnessConfig};
pub use keys::{AgreedKeys, BeaconKeys};
pub use plane::{build, ArtifactSource, Beacon, BeaconConfig};
pub use resolve::{BeaconVerify, KeyLookup};
pub use seed::{constant_fallback_seed, prev_randao_from_seed, witness_fallback_seed, Seed};
pub use surface::{
    absent, for_keys, for_seeds, BeaconResolve, BeaconResolver, PinEffort, Randomness, ShareProbe,
    SignerVerdict, WithheldReason, WitnessCheck,
};

// The crate-internal tier. Same front door, narrower audience — see the boundary
// note above.
pub(crate) use artifact::CommitteeSource;
pub(crate) use carry::frozen_dkg_qual;
pub(crate) use dkg_engine::agreement_partition;
pub(crate) use surface::absent_unregistered;

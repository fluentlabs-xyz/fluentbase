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
//! ONE trait, [`Beacon`], and ONE way to get an implementation of it: the two
//! [`build`] constructors, each of which also hands back the [`Tasks`] the node
//! has to supervise and drain. Every consumer holds `Arc<dyn Beacon>`.
//!
//! Every submodule below is PRIVATE, so the lists in this file are the whole of
//! the module's front door and the compiler is what enforces it: a path like
//! `beacon::certify::SeedStore` from any other file does not resolve. A grep for
//! `beacon::` is no longer the way to audit the boundary — these three lists are.
//!
//! `pub use` — what leaves the module: the [`Beacon`] trait and its vocabulary
//! ([`ShareProbe`], [`SignerVerdict`], [`WithheldReason`], [`PinEffort`],
//! [`Observed`], [`ObservedCertificate`], [`BeaconEvent`], [`DataFault`]), the
//! [`Seed`] wire type with its `prev_randao` derivation, the two constructors
//! with their input and [`Tasks`] types, and the two supplies the node has to
//! make from outside the module: the staking reads ([`CommitteeReads`]) and, for
//! a follower, the one route it has to fetch an artifact over
//! ([`ArtifactFetch`]).
//!
//! Four of the vocabulary names — [`WithheldReason`], [`PinEffort`],
//! [`Observed`], [`DataFault`] — are here because a signature above names them,
//! not because anything outside reads them yet: the first two are fields of
//! [`ShareProbe`]/`ensure_key`, and the last two are the return types of
//! [`Beacon::observe_certificate`] and [`Beacon::faults`]. PLAN rows 5.1-5.2 give
//! the last two their readers.
//!
//! On NEITHER list, and deliberately: how the epoch key is agreed, where
//! the artifact is stored, how a share is derived, how a peer is served, and the
//! module-internal [`surface::Randomness`] trait the two PRODUCTION
//! implementations still speak while the internals move
//! (`.dpos-study/PLAN.md` rows 5.1-5.4). The testbed's own implementations do not
//! speak it: they implement [`Beacon`] directly, so the trait is the one
//! substitution seam and `Randomness` is `pub(super)`.
//!
//! The crate's own TESTS are the one standing exception, and they go through
//! `testing` — one `#[cfg(test)]` door with everything they reach listed in it,
//! rather than a submodule path each. NOT a doc link on purpose: this doc is
//! compiled without `cfg(test)`, where the module it would point at is absent.

mod actor;
mod artifact;
mod carry;
mod ceremony;
mod certify;
mod confirmations;
mod dkg_agree;
mod dkg_engine;
mod dkg_msg;
/// Local single-process DKG oracle — used only by the beacon's own tests (the
/// production path is the networked `actor`/`ceremony`). `#[cfg(test)]`-gated so
/// it is not compiled into release builds.
#[cfg(test)]
mod dkg_oracle;
mod dkg_transport;
mod follower;
mod key_journal;
mod keys;
mod log_resolver;
mod log_store;
mod metrics;
mod oracle;
mod outcome;
mod plane;
mod resolve;
mod seed;
mod seed_journal;
mod share_state;
mod surface;
mod verified_seed;
mod wire;

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
// PRIVATE, not `pub(crate)`: every reader is a submodule of this one, and a child
// module sees its parent's private items. Crate visibility widened the door by an
// element nothing outside `beacon/` has ever named.
const JOURNAL_RETENTION_EPOCHS: u64 = 1;

pub use follower::{build_follower, ArtifactFetch, FollowerInputs};
pub use plane::{build, CommitteeReads, Tasks, ValidatorInputs};
pub use seed::{constant_fallback_seed, prev_randao_from_seed, witness_fallback_seed, Seed};
pub use surface::{
    Beacon, BeaconEvent, DataFault, Observed, ObservedCertificate, PinEffort, ShareProbe,
    SignerVerdict, WithheldReason,
};

// The crate-internal tier. Same front door, narrower audience — see the boundary
// note above.
pub(crate) use dkg_engine::agreement_partition;
pub(crate) use surface::absent_unregistered;

/// The crate's own TEST tier — the ONE door the crate's tests reach beacon
/// internals through, now that the submodules are private.
///
/// It exists because the fixtures of `executor`, `epoch_manager`, `cert_inlet`,
/// `dpos`, `application`, `slasher` and the testbed build their beacons out of
/// the REAL rungs (a real [`certify::SeedStore`], a real [`keys::BeaconKeys`],
/// the shipped [`surface::LiveBeacon`]) rather than out of stubs that agree with
/// them today. Those tests live in the same FILES as the production code they
/// cover, so a grep for `beacon::` cannot tell a test reach from a production
/// one — but the compiler can, because everything below is `#[cfg(test)]`: a
/// production line that reaches for any of it does not compile.
///
/// Additions here are a cost, not a convenience. Each one is an internal that
/// the rows 5.1-5.4 now have to keep nameable while they move it.
#[cfg(test)]
pub(crate) mod testing {
    pub(crate) use super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
    pub(crate) use super::artifact::{decode_artifact, ArtifactStore};
    pub(crate) use super::carry::DkgQualFor;
    pub(crate) use super::certify::SeedStore;
    pub(crate) use super::keys::{AgreedKeyAt, AgreedKeys, BeaconKeys, KeySource, KeySources};
    pub(crate) use super::metrics::BeaconMetrics;
    pub(crate) use super::outcome::{encode_outcome, group_public_key, parse_outcome, DkgOutcome};
    pub(crate) use super::surface::testing::Canned;
    pub(crate) use super::surface::{
        absent, for_seeds, BeaconResolve, DealtOracle, LiveBeacon, LiveBeaconConfig,
        StaticRandomness,
    };
    pub(crate) use super::verified_seed::{PkOracle, VerifiedSeed};
    /// The byzantine roles' tier: only `testbed::byzantine_roles` names these, so
    /// they are gated as it is — an ungated re-export is an unused import in a
    /// stand built without the feature.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub(crate) use super::{
        actor::CommitteeFor,
        ceremony::info_for,
        dkg_msg::{DealerReveal, DkgBody, DkgMsg},
        wire::BeaconMessage,
    };
}

//! Per-epoch threshold randomness beacon: a separate BLS12-381 threshold key
//! (consensus stays multisig for attributability) that committee[E] deals to
//! itself (Model B: dealers == players) via a commonware Joint-Feldman self-DKG
//! run during epoch E-1.
//!
//! The per-round seed is the recovered threshold signature over
//! `(seed_namespace ‖ round)`. It rides inside the combined consensus scheme: the
//! seed partial is carried in each Notarize/Finalize vote and recovered from the
//! round's certificate (notarization or finalization), so it is available at
//! notarization, where speculative execution consumes it.
//! `prev_randao(R) = H(seed(R))` for the block ordered at round R (see
//! [`prev_randao_from_seed`]). The seed is unique by construction (any ≥t partials
//! recover the same value) and unpredictable to the transactions of block R.
//!
//! # The boundary
//!
//! One trait, [`Beacon`], and one way to get an implementation: the two [`build`]
//! constructors, each of which also hands back the [`Tasks`] the node supervises
//! and drains. Every consumer holds `Arc<dyn Beacon>`.
//!
//! Every submodule below is private, so the `pub use` list here is the whole of
//! the module's front door and the compiler enforces it. Deliberately off it: how
//! the epoch key is agreed, where the artifact is stored, how a share is derived,
//! how a peer is served, and the module-internal [`surface::Randomness`] trait,
//! which has one production implementation left.
//!
//! The crate's own tests go through the `#[cfg(test)]` `testing` module — one door
//! with everything they reach listed in it.

mod actor;
mod artifact;
mod ceremony;
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
mod log_resolver;
mod log_store;
mod metrics;
mod oracle;
mod outcome;
mod plane;
mod seed;
mod seed_index;
mod seed_journal;
mod share_state;
mod surface;
mod verified_seed;
mod wire;

// The one retention window every per-epoch map in this module ages out on. It
// lives here, not in the module that happens to sweep on it: `share_state` needs
// the same number for its on-disk reconcile. Under-retention is safe — a member
// that finds the logs evicted keeps fetching or stays a verify-only observer and
// never adopts a wrong share — so widening buys heal reach and costs disk,
// including the secret `ReceivedDealing` views that live at rest under the same
// predicate.
//
// Operational monitor: `epoch_engine_demoted_no_polynomial` persisting for more
// than the window for one identity means a demote whose logs aged out before
// catch-up.
/// Trailing epochs past its own boundary for which a finalized/stalled epoch's DKG
/// journal (own `ReceivedDealing` views and the shared QUAL logs) and the
/// dealer-log serve cache ([`log_store::DealerLogStore`]) are retained — the
/// recompute-heal window. A demoted `committee[E]` member, or a peer it serves,
/// recomputes E's share from these while E is still committee-relevant.
///
/// The same window as the scheme registry's; the comment above says why.
const JOURNAL_RETENTION_EPOCHS: u64 = crate::SCHEME_RETENTION_EPOCHS as u64;

// The four on-disk names the plane opens. They live here, in the module that
// opens them, and nowhere else: no file outside `beacon/` reads any of them, so
// none is re-exported. Every one is `{partition_prefix}` ‖ name — production
// passes the empty prefix (`plane::journal_partition`).

/// Partition for the durable `Round → σ` store behind the
/// [`seed_index::SeedIndex`]. Not under the marshal's partition prefix: this is a
/// Fluent-side store beside the marshal's, not part of it, and it must stay
/// independently prunable.
///
/// The name was changed from `beacon-seed-journal` when the backing primitive
/// moved from `journal::segmented::fixed` to `ordinal::Ordinal`; the two on-disk
/// formats are incompatible. No block body couriers σ any more, so a cold store
/// costs the replay its first source and pushes it onto the local certificate,
/// then the upstream, then a defer. A further format change needs a real
/// migration, not a rename.
const SEED_JOURNAL_PARTITION: &str = "beacon-seed-ordinal";

/// Partition of the durable mint memo (`epoch → the epoch that minted the key in
/// force at it`, [`artifact::MintIndex`]). Empty means RAM-only.
///
/// Not the old `beacon-key-ordinal`: that name belonged to the deleted
/// `epoch → PK_epoch` key journal, a different record and codec. A `Metadata`
/// opened over a partition holding another codec's blobs panics at init, so the
/// two formats get two names.
const MINT_MEMO_PARTITION: &str = "beacon-mint-metadata";

/// Partition of the durable `epoch → agreement artifact` store
/// ([`artifact::ArtifactStore`]), opened once per process by [`build`] — a second
/// handle over this partition would be a dual-writer.
const ARTIFACT_JOURNAL_PARTITION: &str = "beacon-artifact-metadata";

/// Base name of the epoch-key agreement instance's journal partition:
/// `{partition_prefix}dkg_epoch_{target_epoch}`. One per target epoch, holding the
/// simplex voter's journal for the life of the instance.
///
/// Owned by the beacon's agreement launcher, which is also what sweeps it: the
/// instance destroys its own partition after it delivers, and every partition an
/// abort left behind is reclaimed by the launcher's band sweep
/// (`dkg_engine::prune_agreements`). Nothing outside the beacon opens, names or
/// removes one.
const AGREEMENT_JOURNAL_PARTITION_PREFIX: &str = "dkg_epoch_";

pub use plane::{build, CommitteeReads, Tasks, ValidatorInputs};
pub use plane::{build_follower, ArtifactFetch, FollowerInputs};
pub use seed::{constant_fallback_seed, prev_randao_from_seed, witness_fallback_seed, Seed};
pub use surface::{
    Beacon, BeaconEvent, DataFault, Observed, ObservedCertificate, PinEffort, ShareProbe,
    SignerVerdict, WithheldReason,
};

// The crate-internal tier. Same front door, narrower audience — see the boundary
// note above.
pub(crate) use surface::absent_unregistered;

/// The crate's own test tier — the one door the crate's tests reach beacon
/// internals through, now that the submodules are private.
///
/// The fixtures of `executor`, `epoch_manager`, `cert_inlet`, `dpos`,
/// `application`, `slasher` and the testbed build their beacons out of the real
/// rungs rather than out of stubs. Those tests live in the same files as the
/// production code they cover, so a grep cannot tell a test reach from a
/// production one, but the compiler can: everything below is `#[cfg(test)]`.
///
/// Additions here are a cost, not a convenience: each one is an internal that has
/// to stay nameable while the module is reworked.
#[cfg(test)]
pub(crate) mod testing {
    /// The `channel` label the beacon's refusals carry on the shared
    /// `dpos_ingress_dropped_total` — the stand tests filter on it, so a `reason`
    /// count is the beacon's and not another gated channel's.
    pub(crate) use super::actor::BEACON_CHANNEL_LABEL;
    pub(crate) use super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
    /// The seal margin, so the stand's catch-up dealer test
    /// (`testbed::cert_inlet_tests`) can name the deal window in the actor's
    /// own terms rather than as a number that happens to agree with it.
    pub(crate) use super::actor::DKG_MARGIN_BLOCKS;
    pub(crate) use super::artifact::{
        artifact_with_key, decode_artifact, AcquireArtifact, AcquireMint, ArtifactStore,
        MintFixture,
    };
    pub(crate) use super::metrics::BeaconMetrics;
    pub(crate) use super::outcome::{encode_outcome, group_public_key, parse_outcome, DkgOutcome};
    /// The old name of [`seed_index::SeedIndex`] survives as an alias here and
    /// nowhere else, because `epoch_manager.rs`'s test module still names it.
    /// Dropping the alias is one rename in that file.
    pub(crate) use super::seed_index::SeedIndex as SeedStore;
    pub(crate) use super::surface::testing::Canned;
    /// An empty [`super::artifact::KeyIndex`], so a fixture can build a real
    /// [`super::surface::LiveBeacon`] over a real [`super::seed_index::SeedIndex`]
    /// whose every epoch answers `NoKey`.
    pub(crate) use super::surface::{
        absent, keyless_index, DealtOracle, LiveBeacon, LiveBeaconConfig, StaticRandomness,
    };
    pub(crate) use super::verified_seed::{PkOracle, VerifiedSeed};
    /// The byzantine roles' tier: only `testbed::byzantine_roles` names these, so
    /// they are gated as it is — an ungated re-export is an unused import in a
    /// stand built without the feature.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub(crate) use super::{actor::CommitteeFor, ceremony::info_for, dkg_msg::DealerReveal};
    /// The stand's `Role::StrayDealer` builds one real `Commitment` dealing and
    /// frames it as the actor would, so the gate it is refused at is tested with a
    /// frame the actor could have consumed — hence ungated.
    pub(crate) use super::{
        ceremony::DkgCeremony,
        dkg_msg::{DkgBody, DkgMsg},
        wire::BeaconMessage,
    };
}

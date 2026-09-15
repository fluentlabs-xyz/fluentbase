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
//! `beacon::seed_index::SeedIndex` from any other file does not resolve. A grep for
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
//! TWO of the vocabulary names — [`WithheldReason`], [`PinEffort`] — are here
//! because a signature above names them, not because anything outside reads them
//! yet: both are fields of [`ShareProbe`]/`ensure_key`.
//!
//! The other two are READ, and by production. [`Observed`], the verdict of
//! [`Beacon::observe_certificate`], is branched on at three production sites plus
//! the crash-survivor replay: the notarization door decides whether to speculate
//! at all (`spec_exec.rs:123`), the live-stream cert inlet turns a `Refused` into
//! a counted data fault (`cert_inlet.rs:733-736`), the by-height door
//! (`cert_inlet::UpstreamResolver`) makes it a loud witness with no lever
//! (`cert_inlet.rs:3030-3032`), and the replay's `seed_via_beacon` maps all four
//! variants onto its `CertSeed` answer (`dpos.rs:750-758`). [`DataFault`], the
//! item of [`Beacon::faults`], is the inlet's late-verdict charge: it holds the
//! receiver (`cert_inlet.rs:406`, taken at `:472`) and drains it into the
//! rotate judgement on every certificate (`drain_late_verdicts`,
//! `cert_inlet.rs:562`, `:847`).
//!
//! On NEITHER list, and deliberately: how the epoch key is agreed, where
//! the artifact is stored, how a share is derived, how a peer is served, and the
//! module-internal [`surface::Randomness`] trait, which by now has exactly ONE
//! production implementation left — `impl Randomness for LiveBeacon`
//! (`surface.rs:2194`), the single hit of `git grep 'impl Randomness for'
//! -- crates` — and which stays a trait while the internals move
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
// lives HERE, not in the module that happens to sweep on it: `share_state` needs
// the same number for its on-disk reconcile, and reaching sideways into `actor`
// for it made a policy constant look like an actor detail.
//
// AND IT IS NO LONGER A NUMBER OF ITS OWN (§5.3 of
// `.dpos-study/history/E5-BEACON-DESIGN.md`): the DKG journal, the dealer-log serve
// cache and the recompute-heal window all take
// [`crate::SCHEME_RETENTION_EPOCHS`], the one window this crate already measures
// "epochs whose certificates are still verified" in. Two windows meant two answers
// to one question — how far back is an epoch still committee-relevant — and the
// narrower one (`1`) was the BELTED default rather than a derived bound: its own doc
// derived a wider one, `>= ceil(worst_case_catchup_seconds / epoch_seconds) + 1`.
//
// THE DIRECTION IS SAFE AND THE CODE SAYS SO, which is why this is a merge and not a
// re-derivation: "Under-retention is SAFE: a member that finds the logs evicted
// simply keeps fetching / stays a verify-only observer — it never adopts a wrong
// share (the recompute self-check gates that), so widening only trades disk for heal
// reach." Widening 1 → 8 therefore buys heal reach and costs disk: the retained set
// is `window × (~430 KiB QUAL set at n=51 + the per-dealer secret view bodies)` per
// epoch, so ~3.4 MB of QUAL sets at n=51 against ~430 KiB, plus the secret views —
// and the SECRET half is why this is worth stating rather than waving through: eight
// epochs of `ReceivedDealing` views live at rest instead of one, all under 0600 and
// all swept by the same predicate.
//
// Operational monitor, unchanged: `epoch_engine_demoted_no_polynomial` persisting
// for more than the window for one identity = a demote whose logs aged out before
// catch-up.
//
// The name stays because the READERS are the journal's, not the scheme registry's —
// `log_store`'s doc link and `share_state::reconcile_journals` both name the journal
// window specifically — and an alias whose value is the crate constant is one policy
// under two names rather than two policies.
/// Trailing epochs past its own boundary for which a finalized/stalled epoch's DKG
/// journal (own `ReceivedDealing` views AND the shared QUAL logs) + the dealer-log
/// serve cache ([`log_store::DealerLogStore`]) are RETAINED — the recompute-heal
/// window (§8.11.1). A demoted `committee[E]` member (or a peer it serves) recomputes
/// E's share from these while E is still committee-relevant, instead of being swept
/// the instant `now == E` and lingering a verify-only observer until the next
/// committee change.
///
/// ONE window with the scheme registry's, and the comment above says why.
const JOURNAL_RETENTION_EPOCHS: u64 = crate::SCHEME_RETENTION_EPOCHS as u64;

// The four on-disk names the plane opens (E5-40). They live HERE, in the module
// that opens them, and nowhere else: no file outside `beacon/` reads any of them
// (`git grep` on each name is empty outside this directory), so none is
// re-exported. Every one is `{partition_prefix}` ‖ name — production passes the
// empty prefix (`plane::journal_partition`).

/// Partition for the durable `Round → σ` store behind the
/// [`seed_index::SeedIndex`]. Deliberately NOT under the marshal's partition
/// prefix (`consensus_marshal`): this is a Fluent-side store beside the marshal's,
/// not part of it, and it must stay independently prunable.
///
/// Renamed from `beacon-seed-journal` when the backing primitive moved from
/// `journal::segmented::fixed` to `ordinal::Ordinal`: the two on-disk formats are
/// incompatible, and pointing at a fresh name lets the retention window simply
/// refill.
///
/// That refill is no longer free. No block body couriers σ any more — the live
/// derive and the crash-survivor replay both key on the block's own round — so a
/// cold store costs the replay its first source and pushes it onto the local
/// certificate, then the upstream, then a defer. **A further format change needs
/// a real migration, not a rename.**
const SEED_JOURNAL_PARTITION: &str = "beacon-seed-ordinal";

/// Partition of the durable mint memo (`epoch → the epoch that MINTED the key in
/// force at it`, [`artifact::MintIndex`]). Empty would mean RAM-only.
///
/// NOT the old `beacon-key-ordinal`, and the rename is not cosmetic: that name
/// belonged to the deleted `epoch → PK_epoch` key journal, whose backing primitive
/// was `ordinal::Ordinal`, while this is a `Metadata` store of a different record.
/// A `Metadata` opened over a partition holding another codec's blobs PANICS at
/// init (`.claude/COMMONWARE_INTERNALS.md`, "wrong codec on an existing
/// partition"), and "every net relaunches from a fresh genesis" is a deployment
/// policy, not a property of this code — so the two formats get two names.
const MINT_MEMO_PARTITION: &str = "beacon-mint-metadata";

/// Partition of the durable `epoch → agreement artifact` store
/// ([`artifact::ArtifactStore`]), opened once per process by [`build`] — a second
/// handle over this partition would be a dual-writer.
const ARTIFACT_JOURNAL_PARTITION: &str = "beacon-artifact-metadata";

/// Base name of the epoch-key agreement instance's journal partition:
/// `{partition_prefix}dkg_epoch_{target_epoch}` (`dkg_engine`, the private
/// `agreement_partition`). One per target epoch, holding the simplex voter's
/// journal for the life of the instance.
///
/// OWNED by the beacon's agreement launcher, which is also what SWEEPS it: the
/// instance destroys its own partition after it delivers, and every partition an
/// abort left behind is reclaimed by the launcher's band sweep — the
/// `SCHEME_RETENTION_EPOCHS` targets below the actor's epoch clock, by epoch
/// number, on the edge the clock moves (`dkg_engine::prune_agreements`). Nothing
/// outside the beacon opens, names or removes one.
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

/// The crate's own TEST tier — the ONE door the crate's tests reach beacon
/// internals through, now that the submodules are private.
///
/// It exists because the fixtures of `executor`, `epoch_manager`, `cert_inlet`,
/// `dpos`, `application`, `slasher` and the testbed build their beacons out of
/// the REAL rungs (a real [`seed_index::SeedIndex`], a real
/// [`artifact::KeyIndex`] over a real [`artifact::ArtifactStore`], the shipped
/// [`surface::LiveBeacon`]) rather than out of stubs that agree with
/// them today. Those tests live in the same FILES as the production code they
/// cover, so a grep for `beacon::` cannot tell a test reach from a production
/// one — but the compiler can, because everything below is `#[cfg(test)]`: a
/// production line that reaches for any of it does not compile.
///
/// Additions here are a cost, not a convenience. Each one is an internal that
/// the rows 5.1-5.4 now have to keep nameable while they move it.
#[cfg(test)]
pub(crate) mod testing {
    /// The `channel` label the beacon's refusals carry on the shared
    /// `dpos_ingress_dropped_total` — the stand tests filter on it, so that a
    /// `reason` count is the beacon's and not another gated channel's (5.3-В,
    /// third round).
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
    /// Row 5.2 renamed this type (`SeedStore` → [`seed_index::SeedIndex`]) and
    /// folded the quarantine into it as a state. The OLD NAME survives as an alias
    /// HERE and nowhere else, because `epoch_manager.rs`'s test module names it
    /// (`epoch_manager.rs:2076`, `:2080`, `:2129`, `:2836`, `:2848`) and that file
    /// is outside row 5.2's write list. Dropping the alias is one rename in that
    /// file.
    pub(crate) use super::seed_index::SeedIndex as SeedStore;
    pub(crate) use super::surface::testing::Canned;
    /// `keyless_index` is row 5.2's replacement for the deleted `for_seeds`: an
    /// empty [`super::artifact::KeyIndex`], so a fixture can build a real
    /// [`super::surface::LiveBeacon`] over a real [`super::seed_index::SeedIndex`]
    /// whose
    /// every epoch answers `NoKey`. It is a CONSTRUCTOR on the test boundary, not
    /// a rename of anything above — unlike the `SeedStore` alias above it.
    pub(crate) use super::surface::{
        absent, keyless_index, DealtOracle, LiveBeacon, LiveBeaconConfig, StaticRandomness,
    };
    pub(crate) use super::verified_seed::{PkOracle, VerifiedSeed};
    /// The byzantine roles' tier: only `testbed::byzantine_roles` names these, so
    /// they are gated as it is — an ungated re-export is an unused import in a
    /// stand built without the feature.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub(crate) use super::{actor::CommitteeFor, ceremony::info_for, dkg_msg::DealerReveal};
    /// The stand's `Role::StrayDealer` (5.3-В) builds one real `Commitment`
    /// dealing and frames it as the actor would, so the gate it is refused at is
    /// tested with a frame the actor could have consumed — hence ungated.
    pub(crate) use super::{
        ceremony::DkgCeremony,
        dkg_msg::{DkgBody, DkgMsg},
        wire::BeaconMessage,
    };
}

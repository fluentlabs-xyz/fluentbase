//! The threshold half of the consensus scheme, behind a synchronous interface.
//!
//! The scheme holds no key material: it delegates every threshold operation to
//! an implementation of [`SeedOracle`] supplied by the beacon, so the share, the
//! public polynomial and `PK_epoch` never cross into this crate.
//!
//! The trait is declared here rather than beside its implementation because the
//! dependency runs one way only — `fluentbase-consensus` depends on
//! `fluentbase-bls`, never the reverse.

use commonware_consensus::types::Round;
use commonware_utils::Participant;

use crate::BlsSignature;

/// Outcome of verifying an assembled seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedCheck {
    Valid,
    Invalid,
    /// The epoch key is not resolvable locally yet. The caller admits the
    /// certificate on its multisig half alone and nobody consumes its σ. The
    /// oracle takes NO custody of the value.
    NoKey,
}

/// The beacon's synchronous face for everything threshold.
///
/// SYNC BY CONTRACT: every method is called inline from the simplex batcher
/// (`verify_attestations` loops per attestation) and from the sign/assemble
/// paths. An implementation MUST NOT block on I/O and MUST NOT await; it reads
/// locally held material under a lock and answers. Acquisition happens
/// elsewhere, out of band.
///
/// Every method takes a [`Round`], which carries the epoch: the caller checks
/// the epoch binding before delegating, so an implementation may trust
/// `round.epoch()`.
pub trait SeedOracle: std::fmt::Debug + Send + Sync + 'static {
    /// This node's partial for `round`. `None` ⇒ no usable share for the epoch.
    fn sign_partial(&self, round: Round) -> Option<BlsSignature>;

    /// Verify one partial from participant `index`.
    fn verify_partial(&self, round: Round, index: Participant, value: &BlsSignature) -> bool;

    /// Recover σ from `partials`. `threshold` is computed by the CALLER from the
    /// same `M: Faults` the vote quorum used — that is what keeps the seed
    /// threshold and the vote quorum in lockstep (see
    /// [`crate::beacon::recover_seed_with_threshold`]).
    ///
    /// Takes no `Round`, unlike its siblings, and cannot: `CertScheme::assemble`
    /// is handed attestations alone — no subject, hence no round. Recovery does
    /// not need one either, being interpolation over the partials; only SIGNING
    /// and VERIFYING a partial are message-bound.
    fn recover(
        &self,
        partials: &[(Participant, BlsSignature)],
        threshold: u32,
    ) -> Option<BlsSignature>;

    /// Verify an assembled σ against `PK_e`. [`SeedCheck::NoKey`] is a pure
    /// answer: the implementation stores nothing and has no side effect of any
    /// kind.
    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck;
}

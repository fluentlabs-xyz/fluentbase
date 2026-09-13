//! The witness that a σ was checked against its epoch key.
//!
//! Until this ticket the seed store had exactly one writer — this node's own
//! notarization reporter — so "every stored σ is correct" needed no enforcement.
//! Ingress capture and the by-round transport each add a writer of a σ this node
//! did not produce, and the asymmetry those writers introduce is not symmetric:
//! a MISSING σ costs a skipped view, while a WRONG σ reaches `prev_randao` and
//! therefore the state root, which is a fork.
//!
//! So the check is made unforgeable by the type system rather than by a
//! convention each writer must remember: [`VerifiedSeed`] has private fields and
//! no public constructor that skips [`SeedOracle::verify_seed`], and every
//! insertion path into the served map takes one.

use commonware_consensus::types::Round;
use fluentbase_bls::{
    oracle::{SeedCheck, SeedOracle},
    BlsSignature,
};

/// A σ that verified against `PK_e` for the round it is bound to.
///
/// Carries its own round: the round is what σ is a signature OVER, so a witness
/// separated from it would let a caller file a valid signature under the wrong
/// key. Consumers read both out of the one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedSeed {
    round: Round,
    seed: BlsSignature,
}

impl VerifiedSeed {
    /// Check `seed` against the epoch key `oracle` holds, and mint the witness
    /// only on [`SeedCheck::Valid`].
    ///
    /// The error arm hands back the oracle's own answer because the two failures
    /// are not the same event and must not be handled alike:
    /// [`SeedCheck::Invalid`] is a peer serving a σ that does not verify against
    /// an attested key — a real witness, and loud. [`SeedCheck::NoKey`] is a
    /// statement about US, not about the network: the epoch key is not resolvable
    /// here yet, the value is HELD (`Pending`), and it is re-checked when the key
    /// lands.
    pub fn check(
        oracle: &dyn SeedOracle,
        round: Round,
        seed: BlsSignature,
    ) -> Result<Self, SeedCheck> {
        match oracle.verify_seed(round, &seed) {
            SeedCheck::Valid => Ok(Self { round, seed }),
            other => Err(other),
        }
    }

    /// Rebuild the witness for an entry replayed from the seed journal.
    ///
    /// This is the ONE place a witness is reconstructed rather than checked, and
    /// it is sound only while the durable half cannot receive an unchecked σ.
    /// Three feeders reach that partition and all three are named here on
    /// purpose — a fourth one appearing without this doc changing is exactly the
    /// silent regression the type is meant to prevent:
    ///
    /// 1. the persist channel, whose sender is created inside
    ///    [`SeedIndex::with_persistence`](crate::beacon::seed_index::SeedIndex::with_persistence)
    ///    and therefore cannot exist outside an index whose only writer takes a
    ///    `VerifiedSeed`;
    /// 2. [`SeedJournal::append`](crate::beacon::seed_journal::SeedJournal::append);
    /// 3. [`spawn_writer`](crate::beacon::seed_journal::spawn_writer), which owns
    ///    the receiving half of (1).
    ///
    /// Re-checking at replay instead is not an option and not merely an
    /// optimisation: epoch keys are pruned to a trailing
    /// `SCHEME_RETENTION_EPOCHS` window while the σ window is measured in
    /// ROUNDS, so the older part of a legitimately written window has no key by
    /// policy. Verification-as-a-gate would discard correct entries.
    pub(crate) fn from_journal(round: Round, seed: BlsSignature) -> Self {
        Self { round, seed }
    }

    pub fn round(&self) -> Round {
        self.round
    }

    pub fn seed(&self) -> BlsSignature {
        self.seed
    }
}

/// A [`SeedOracle`] that holds one group public key and answers nothing else.
///
/// Test scaffolding for the many call sites that produce a genuine threshold σ
/// from a dealt committee and then need a witness for it. The production oracles
/// reach a key store, a ceremony and a metrics registry; none of that is what
/// those tests are about.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct PkOracle {
    pk: fluentbase_bls::beacon::GroupPublic,
    namespace: Vec<u8>,
}

#[cfg(test)]
impl PkOracle {
    pub(crate) fn new(pk: fluentbase_bls::beacon::GroupPublic, namespace: Vec<u8>) -> Self {
        Self { pk, namespace }
    }

    /// Mint the witness for a σ this fixture just produced.
    pub(crate) fn witness(&self, round: Round, seed: BlsSignature) -> VerifiedSeed {
        VerifiedSeed::check(self, round, seed).expect("fixture seed verifies under its own key")
    }
}

#[cfg(test)]
impl SeedOracle for PkOracle {
    fn sign_partial(&self, _round: Round) -> Option<BlsSignature> {
        None
    }

    fn verify_partial(
        &self,
        _round: Round,
        _index: commonware_utils::Participant,
        _value: &BlsSignature,
    ) -> bool {
        false
    }

    fn recover(
        &self,
        _partials: &[(commonware_utils::Participant, BlsSignature)],
        _threshold: u32,
    ) -> Option<BlsSignature> {
        None
    }

    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
        if fluentbase_bls::beacon::verify_seed(&self.pk, &self.namespace, round, seed) {
            SeedCheck::Valid
        } else {
            SeedCheck::Invalid
        }
    }
}

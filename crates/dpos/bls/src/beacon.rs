//! Threshold randomness seed primitives (BLS12-381 MinSig), shared by the
//! combined consensus scheme and the deriver.
//!
//! The seed for a round is the unique recovered threshold signature over
//! `round.encode()` — any ≥ t partials recover the same value. The
//! `prev_randao = keccak256(signature)` derivation lives in the consumer crate;
//! this module is pure BLS, no alloy.

use commonware_codec::Encode as _;
use commonware_consensus::types::Round;
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    ops,
    ops::threshold,
    sharing::Sharing,
    variant::{MinSig, PartialSignature},
    Error,
};
use commonware_parallel::Sequential;
use commonware_utils::{Faults, N3f1};

use crate::BlsSignature;

/// Domain separator suffix for beacon seed signatures, distinct from the
/// consensus vote and proof-of-possession namespaces so a seed can never be
/// replayed as a consensus signature.
const BEACON_SEED_SUFFIX: &[u8] = b"_BEACON_SEED";

/// Domain separator suffix for the epoch-key agreement instance.
const DKG_AGREE_SUFFIX: &[u8] = b"_DKG_AGREE";

/// The group public key `PK_epoch` a verifier checks recovered seeds against.
pub type GroupPublic =
    <MinSig as commonware_cryptography::bls12381::primitives::variant::Variant>::Public;

/// The seed signing namespace for a chain: `chain_namespace ‖ "_BEACON_SEED"`.
pub fn seed_namespace(chain_namespace: &[u8]) -> Vec<u8> {
    let mut ns = Vec::with_capacity(chain_namespace.len() + BEACON_SEED_SUFFIX.len());
    ns.extend_from_slice(chain_namespace);
    ns.extend_from_slice(BEACON_SEED_SUFFIX);
    ns
}

/// The signing namespace for the epoch-key agreement instance — a second
/// `simplex` running over `committee[E+1]` during epoch `E`.
///
/// It must be distinct from the chain namespace and from every other namespace
/// derived from it. `union_unique` length-prefixes the namespace, so distinctness is
/// enough; under a shared base an honest validator's agreement vote at
/// `Round(E+1, v)` and its ordering vote at that same round would be two payloads
/// from one signer at one round — the shape equivocation evidence is extracted
/// from — and evidence submission is permissionless, so any observer of the DKG
/// sub-channel could slash that validator.
pub fn dkg_namespace(chain_namespace: &[u8]) -> Vec<u8> {
    let mut ns = Vec::with_capacity(chain_namespace.len() + DKG_AGREE_SUFFIX.len());
    ns.extend_from_slice(chain_namespace);
    ns.extend_from_slice(DKG_AGREE_SUFFIX);
    ns
}

/// The message signed for a consensus round (epoch ‖ view, canonical codec
/// encoding). The seed is keyed by round, not height: the consumer carries the
/// round from the finalization certificate, and among finalized blocks height and
/// round are 1:1, so the round is a sound key.
fn seed_message(round: Round) -> Vec<u8> {
    round.encode().to_vec()
}

pub fn sign_seed_partial(
    share: &Share,
    namespace: &[u8],
    round: Round,
) -> PartialSignature<MinSig> {
    threshold::sign_message::<MinSig>(share, namespace, &seed_message(round))
}

/// Verify a single partial against the public polynomial (used while collecting
/// partials, to drop invalid contributions before recovery).
pub fn verify_seed_partial(
    sharing: &Sharing<MinSig>,
    namespace: &[u8],
    round: Round,
    partial: &PartialSignature<MinSig>,
) -> bool {
    threshold::verify_message::<MinSig>(sharing, namespace, &seed_message(round), partial).is_ok()
}

/// Recover the unique threshold seed signature for a round from ≥ t verified
/// partials; the consumer pairs the raw signature with the round to derive
/// `prev_randao`.
///
/// Generic over the fault model `M` so the seed quorum stays in lockstep with the
/// vote quorum `CombinedScheme::assemble` recovers under the same `M`: pinning a
/// literal here would let the two halves of one certificate silently disagree.
pub fn recover_seed<M: Faults>(
    sharing: &Sharing<MinSig>,
    partials: &[PartialSignature<MinSig>],
) -> Result<BlsSignature, Error> {
    threshold::recover::<MinSig, _, M>(sharing, partials, &Sequential)
}

/// [`recover_seed`] for a caller that cannot carry `M`: the object-safe
/// [`crate::oracle::SeedOracle`] path, where the threshold arrives as a value
/// rather than as a type.
///
/// The caller owes the lockstep the generic parameter enforces above: `threshold`
/// must be `M::quorum(n)` for the same `M` the vote quorum was counted under. It is
/// checked against the sharing's own quorum rather than trusted, and the equality is
/// two assertions in one: `M == N3f1` (commonware derives the point count from `M`,
/// so the `N3f1` below is what selects it) and the sharing's total being the epoch's
/// consensus committee size (the ceremony dealt to exactly that committee).
/// Comparing fault models alone would drop the second.
///
/// A mismatch is otherwise indistinguishable from a quorum not yet reached, so the
/// caller owes it a log; the per-epoch latch lives in `BeaconOracle::recover`,
/// because a free function has no latch and a `static` here would mute the next
/// epoch's occurrence.
pub fn recover_seed_with_threshold(
    sharing: &Sharing<MinSig>,
    partials: &[PartialSignature<MinSig>],
    threshold: u32,
) -> Result<BlsSignature, Error> {
    if threshold != sharing.required::<N3f1>() {
        return Err(Error::InvalidRecovery);
    }
    threshold::recover::<MinSig, _, N3f1>(sharing, partials, &Sequential)
}

/// Verify a recovered seed signature against the group public key `PK_epoch` —
/// the only check a verifier-only node (no share / no polynomial) can run.
pub fn verify_seed(
    group_public: &GroupPublic,
    namespace: &[u8],
    round: Round,
    signature: &BlsSignature,
) -> bool {
    ops::verify_message::<MinSig>(group_public, namespace, &seed_message(round), signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fluent_namespace, PeerPubkey};
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::{
        bls12381::{dkg::deal, primitives::sharing::Mode},
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::ordered::Set;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// A four-member committee dealing to itself, as the beacon's ceremony does.
    fn dealt(n: usize) -> (Sharing<MinSig>, Vec<Share>) {
        let mut rng = StdRng::seed_from_u64(0xB1A5);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players.clone())
                .expect("deal");
        let held = players
            .iter()
            .map(|p| shares.get_value(p).expect("share").clone())
            .collect();
        (outcome.public().clone(), held)
    }

    /// The lockstep [`recover_seed`] carries by type and this value-taking sibling
    /// can only check here: a threshold that is not the sharing's own quorum must
    /// fail the recovery rather than interpolate over a different point count.
    #[test]
    fn recover_with_threshold_agrees_with_the_generic_and_refuses_a_foreign_quorum() {
        let (sharing, shares) = dealt(4);
        let ns = seed_namespace(&fluent_namespace(20994));
        let round = Round::new(Epoch::new(7), View::new(3));
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        let quorum = N3f1::quorum(shares.len() as u32);

        let by_value =
            recover_seed_with_threshold(&sharing, &partials, quorum).expect("recover by value");
        assert_eq!(
            by_value,
            recover_seed::<N3f1>(&sharing, &partials).expect("recover by type")
        );
        assert!(verify_seed(sharing.public(), &ns, round, &by_value));

        assert!(recover_seed_with_threshold(&sharing, &partials, quorum - 1).is_err());
        assert!(recover_seed_with_threshold(&sharing, &partials, quorum + 1).is_err());
    }

    /// What actually separates the derived namespaces: `union_unique`'s length
    /// prefix plus distinct suffixes, not prefix-freedom against the base — which
    /// the construction does not have, since every derivation appends to the base.
    /// The prefix relation that does matter is between the derived namespaces, and
    /// it is asserted below.
    #[test]
    fn derived_namespaces_are_distinct_and_cannot_collide_when_signed() {
        let base = fluent_namespace(20994);
        let dkg = dkg_namespace(&base);
        let seed = seed_namespace(&base);

        assert_ne!(dkg, seed);
        assert!(!seed.starts_with(&dkg));
        assert!(!dkg.starts_with(&seed));

        assert!(dkg.starts_with(&base));
        assert!(seed.starts_with(&base));

        // The pair that a bare `union` would collide: signing `msg` under the DKG
        // namespace, and signing `DKG_AGREE_SUFFIX ‖ msg` under the base.
        let msg = b"round-payload";
        let shifted: Vec<u8> = [DKG_AGREE_SUFFIX, msg].concat();
        assert_eq!(
            commonware_utils::union(&dkg, msg),
            commonware_utils::union(&base, &shifted),
            "the collision this test exists to rule out must be real under `union`"
        );
        assert_ne!(
            commonware_utils::union_unique(&dkg, msg),
            commonware_utils::union_unique(&base, &shifted),
            "the length prefix is what rules it out, not prefix-freedom"
        );
    }
}

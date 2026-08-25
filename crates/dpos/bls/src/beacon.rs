//! Threshold randomness seed primitives (BLS12-381 MinSig), shared by the
//! combined consensus scheme (which signs/recovers the seed partial alongside
//! each vote) and the deriver (which checks a recovered seed against the epoch
//! group key).
//!
//! The seed for a consensus round is the unique recovered threshold signature
//! over `round.encode()` — any ≥t partials recover the same value. The
//! `prev_randao = keccak256(signature)` derivation lives in the consumer crate
//! (it owns the EVM/alloy types); this module is pure BLS, no alloy.

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

/// Domain separator suffix for beacon seed signatures — distinct from the
/// consensus vote and proof-of-possession namespaces so a beacon seed can
/// never be replayed as a consensus signature (or vice versa).
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

/// The signing namespace for the epoch-key AGREEMENT instance — a second
/// `simplex` running over `committee[E+1]` during epoch `E`:
/// `chain_namespace ‖ "_DKG_AGREE"`.
///
/// It MUST be distinct from the chain namespace and from every other namespace
/// derived from it. Distinct is the whole requirement, and it is enough: a signed
/// message is `union_unique(namespace, msg)`, which LENGTH-PREFIXES the namespace
/// (`CW/utils/src/lib.rs:176-185`), so no two `(namespace, message)` pairs collide
/// and the derivation needs no prefix-freedom against the base — which it does not
/// have anyway, since it appends to it. The tuple a simplex scheme signs
/// carries only `Round{epoch, view}` and the payload — nothing identifies the
/// instance — so under a shared base an honest validator's agreement vote at
/// `Round(E+1, v)` and its ordering vote at that same round are two different
/// payloads from one signer at one round: exactly the shape equivocation
/// evidence is extracted from. The hazard is NOT a local self-slash. Evidence
/// submission is permissionless and the pre-submit verifier is rebuilt from the
/// bare chain namespace with no instance discriminator, so ANY observer of the
/// DKG sub-channel can assemble that pair into calldata and slash an honest
/// validator (`fluentbase_consensus::slasher::evidence`).
pub fn dkg_namespace(chain_namespace: &[u8]) -> Vec<u8> {
    let mut ns = Vec::with_capacity(chain_namespace.len() + DKG_AGREE_SUFFIX.len());
    ns.extend_from_slice(chain_namespace);
    ns.extend_from_slice(DKG_AGREE_SUFFIX);
    ns
}

/// The message signed for a given consensus round (epoch ‖ view, canonical
/// codec encoding). The seed is keyed by ROUND, not height: the `Subject` the
/// scheme signs carries the round, and height↔round is 1:1 among finalized
/// blocks (recovered from the finalization cert by the consumer).
fn seed_message(round: Round) -> Vec<u8> {
    round.encode().to_vec()
}

/// Partial-sign the seed for `round` with this member's DKG share.
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

/// Recover the unique threshold seed signature for a round from ≥t verified
/// partials. Returns the raw recovered signature (the consumer pairs it with
/// the round to form a `Seed` and derive `prev_randao`).
///
/// Generic over the fault model `M` so the seed quorum (`sharing.required::<M>()`)
/// stays in lockstep with the vote quorum `CombinedScheme::assemble` recovers
/// under the same `M` — the whole stack is `N3f1` today, but pinning a literal
/// here would let the two halves of one certificate silently disagree.
pub fn recover_seed<M: Faults>(
    sharing: &Sharing<MinSig>,
    partials: &[PartialSignature<MinSig>],
) -> Result<BlsSignature, Error> {
    threshold::recover::<MinSig, _, M>(sharing, partials, &Sequential)
}

/// [`recover_seed`] for a caller that cannot carry `M`: the object-safe
/// [`crate::oracle::SeedOracle`] path, where the threshold has to arrive as a
/// value rather than as a type.
///
/// The CALLER owes the lockstep the generic parameter enforces above —
/// `threshold` must be `M::quorum(n)` for the same `M` the vote quorum was
/// counted under, computed at the `assemble` call site. It is CHECKED against
/// the sharing's own quorum rather than trusted: commonware derives its
/// evaluation count from `M` and offers no entry point that takes a number, so
/// the `N3f1` below is what actually selects the point count.
///
/// **THE EQUALITY IS TWO ASSERTIONS, NOT ONE**, and the second is unstated in
/// its own operands. `threshold` is `M::quorum(vote_committee.len())` while
/// `sharing.required::<N3f1>()` is `N3f1::quorum(sharing.total())`, so the
/// comparison holds only when BOTH `M == N3f1` (the fault model) AND
/// `vote_committee.len() == sharing.total()` (the DKG dealt to exactly the
/// epoch's consensus committee). The latter is a contract-side invariant —
/// `committee[E]` is what the ceremony deals over — enforced outside this repo,
/// so this line is where a violation of it would first become visible. Do not
/// weaken the check to compare fault models alone.
///
/// **THE CALLER OWES THIS FAILURE A LOG, AND THIS FUNCTION CANNOT PROVIDE ONE.**
/// A mismatch is otherwise indistinguishable from a healthy
/// quorum-not-yet-reached: `assemble` returns `None`, the batcher declines and
/// retries as each further attestation arrives, and the node simply stops
/// producing certificates with nothing in its logs — the class of failure where
/// silence costs the most. But the retry is why the log cannot live here. The
/// condition is FROZEN for the epoch while the call sits on the per-certificate
/// path, so it needs a per-epoch latch, and a free function's only option is a
/// process-wide `static` — which would mute the next epoch's genuinely new
/// occurrence. `BeaconOracle::recover` owns the latch (it is already per-epoch)
/// and logs both operands there.
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

    /// The lockstep the generic parameter of [`recover_seed`] carries by type and
    /// this sibling can only carry by value: a threshold that is not the
    /// sharing's own quorum must fail the recovery, never interpolate over a
    /// different point count.
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

    /// What actually separates the derived namespaces, stated as the code has it
    /// rather than as prefix-freedom against the base — which the construction does
    /// NOT have, since every derivation appends to the base. The separation is
    /// `union_unique`'s length prefix plus distinct suffixes, and that is the
    /// stronger property: it makes the `(namespace, message)` pair injective, so
    /// even the one concatenation that would collide under a bare `union` cannot.
    /// The prefix relation that DOES matter is between the derived namespaces, and
    /// it is asserted below — this repo was bitten by exactly that once already
    /// (`fluent/leader/fallback` vs `fluent/seedless-leader` in `weighted_vrf`).
    #[test]
    fn derived_namespaces_are_distinct_and_cannot_collide_when_signed() {
        let base = fluent_namespace(20994);
        let dkg = dkg_namespace(&base);
        let seed = seed_namespace(&base);

        assert_ne!(dkg, seed);
        assert!(!seed.starts_with(&dkg));
        assert!(!dkg.starts_with(&seed));

        // Not prefix-free against the base, by construction.
        assert!(dkg.starts_with(&base));
        assert!(seed.starts_with(&base));

        // The pair that a bare `union` WOULD collide: signing `msg` under the DKG
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

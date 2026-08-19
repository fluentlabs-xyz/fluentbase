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
use commonware_utils::Faults;

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
    use crate::fluent_namespace;

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

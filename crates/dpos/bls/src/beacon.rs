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
use commonware_utils::N3f1;

use crate::BlsSignature;

/// Domain separator suffix for beacon seed signatures — distinct from the
/// consensus vote and proof-of-possession namespaces so a beacon seed can
/// never be replayed as a consensus signature (or vice versa).
const BEACON_SEED_SUFFIX: &[u8] = b"_BEACON_SEED";

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
pub fn recover_seed(
    sharing: &Sharing<MinSig>,
    partials: &[PartialSignature<MinSig>],
) -> Result<BlsSignature, Error> {
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

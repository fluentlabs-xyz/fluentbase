//! Proof-of-Possession signing and verification.
//!
//! We use the stock Commonware function `ops::sign_proof_of_possession::<MinSig>`
//! unchanged. The signed message body is `union_unique(namespace, pubkey.encode())`
//! under the DST `BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_`. The on-chain
//! Solidity verifier reconstructs this byte layout exactly (pinned by
//! `crates/bls/tests/hash_to_g1_conformance.rs`).
//!
//! Address-binding (Sui-style `PoP_msg = pubkey || validator_address`) was
//! considered and rejected: rogue-key safety already comes from PoP, and
//! address binding adds no security here.

use commonware_codec::{DecodeExt, EncodeFixed};
use commonware_cryptography::bls12381::primitives::ops;

use crate::{
    error::Error, keys::ValidatorBlsKeypair, BlsPubkey, BlsSignature, Variant, PUBKEY_BYTES,
    SIGNATURE_BYTES,
};

/// Sign a Proof-of-Possession for `keypair` under `namespace`.
///
/// Returns the 48-byte compressed G1 signature.
pub fn sign_pop(keypair: &ValidatorBlsKeypair, namespace: &[u8]) -> [u8; SIGNATURE_BYTES] {
    let sig: BlsSignature = ops::sign_proof_of_possession::<Variant>(keypair.secret(), namespace);
    // `BlsSignature::SIZE == SIGNATURE_BYTES` for MinSig (G1 compressed);
    // `encode_fixed` asserts the length matches.
    sig.encode_fixed::<SIGNATURE_BYTES>()
}

/// Verify a Proof-of-Possession.
///
/// Decodes `pubkey` and `signature` (including subgroup checks via blst), then
/// re-hashes `union_unique(namespace, pubkey.encode())` to G1 under the PoP
/// DST and checks the pairing equation.
///
/// Returns [`Error::InvalidPubkey`] / [`Error::InvalidSignature`] for malformed
/// point bytes, and [`Error::InvalidPoP`] when the pairing equation fails.
pub fn verify_pop(
    pubkey: &[u8; PUBKEY_BYTES],
    namespace: &[u8],
    signature: &[u8; SIGNATURE_BYTES],
) -> Result<(), Error> {
    let pk = BlsPubkey::decode(pubkey.as_slice()).map_err(|_| Error::InvalidPubkey)?;
    let sig = BlsSignature::decode(signature.as_slice()).map_err(|_| Error::InvalidSignature)?;

    ops::verify_proof_of_possession::<Variant>(&pk, namespace, &sig).map_err(|_| Error::InvalidPoP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluent_namespace;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn kp(seed: u64) -> ValidatorBlsKeypair {
        ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
    }

    #[test]
    fn sign_verify_round_trip() {
        let k = kp(1);
        let ns = fluent_namespace(20994);
        let sig = sign_pop(&k, &ns);
        verify_pop(&k.public_bytes(), &ns, &sig).expect("PoP must verify");
    }

    #[test]
    fn signature_for_one_validator_does_not_verify_for_another() {
        let alice = kp(1);
        let bob = kp(2);
        let ns = fluent_namespace(20994);
        let sig = sign_pop(&alice, &ns);
        assert!(matches!(
            verify_pop(&bob.public_bytes(), &ns, &sig),
            Err(Error::InvalidPoP)
        ));
    }

    #[test]
    fn signature_under_one_chain_id_does_not_verify_under_another() {
        let k = kp(1);
        let sig = sign_pop(&k, &fluent_namespace(1));
        assert!(matches!(
            verify_pop(&k.public_bytes(), &fluent_namespace(2), &sig),
            Err(Error::InvalidPoP)
        ));
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let k = kp(1);
        let ns = fluent_namespace(20994);
        let mut sig = sign_pop(&k, &ns);
        sig[0] ^= 0x01;
        // Tampering may produce either a malformed point (InvalidSignature)
        // or a valid-but-wrong point (InvalidPoP). Both must be rejected.
        match verify_pop(&k.public_bytes(), &ns, &sig) {
            Err(Error::InvalidSignature) | Err(Error::InvalidPoP) => {}
            other => panic!("expected InvalidSignature or InvalidPoP, got {other:?}"),
        }
    }

    #[test]
    fn malformed_pubkey_is_rejected() {
        let k = kp(1);
        let ns = fluent_namespace(20994);
        let sig = sign_pop(&k, &ns);
        let mut bad_pk = k.public_bytes();
        bad_pk[0] = 0xff; // breaks blst compressed-flag encoding
        assert!(matches!(
            verify_pop(&bad_pk, &ns, &sig),
            Err(Error::InvalidPubkey)
        ));
    }
}

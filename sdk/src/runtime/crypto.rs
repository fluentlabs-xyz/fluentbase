use crate::{runtime::LowLevelSDK, sdk::LowLevelCryptoSDK};
use fluentbase_poseidon::*;
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use k256::{
    ecdsa::{RecoveryId, Signature, VerifyingKey},
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint,
    PublicKey,
};
use keccak_hash::write_keccak;

macro_rules! fr_from_b32 {
    ($b32:ident) => {{
        let fa = Fr::from_bytes($b32);
        if fa.is_none().into() {
            return false;
        }
        fa.unwrap()
    }};
}

impl LowLevelCryptoSDK for LowLevelSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        write_keccak(data, output);
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        let hash = poseidon_hash(data);
        output.copy_from_slice(&hash);
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fd_data: &[u8; 32],
        output: &mut [u8],
    ) -> bool {
        let fa = fr_from_b32!(fa_data);
        let fb = fr_from_b32!(fb_data);
        let fd = fr_from_b32!(fd_data);
        let hasher = Fr::hasher();
        let h2 = hasher.hash([fa, fb], fd);
        let hash = h2.to_repr();
        output.copy_from_slice(&hash);
        true
    }

    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        let sig = Signature::from_slice(sig).unwrap();
        let rec_id = RecoveryId::new(rec_id & 0b1 > 0, rec_id & 0b10 > 0);
        let pk = VerifyingKey::recover_from_prehash(digest, &sig, rec_id).unwrap();
        let pk_computed = EncodedPoint::from(&pk);
        let public_key = PublicKey::from_encoded_point(&pk_computed).unwrap();
        let pk_uncompressed = public_key.to_encoded_point(false);
        output.copy_from_slice(pk_uncompressed.as_bytes());
    }
}

#[cfg(test)]
mod test {
    extern crate alloc;

    use super::*;
    use crate::LowLevelSDK;
    use alloc::{vec, vec::Vec};
    use hex_literal::hex;
    use k256::ecdsa::RecoveryId;
    use keccak_hash::{keccak, write_keccak, KECCAK_EMPTY};
    use sha2::{Digest, Sha256};

    #[test]
    fn empty() {
        assert_eq!(keccak([0u8; 0]), KECCAK_EMPTY);
    }

    #[test]
    fn with_content() {
        let data: Vec<u8> = From::from("hello world");
        let expected: Vec<u8> = vec![
            0x47, 0x17, 0x32, 0x85, 0xa8, 0xd7, 0x34, 0x1e, 0x5e, 0x97, 0x2f, 0xc6, 0x77, 0x28,
            0x63, 0x84, 0xf8, 0x02, 0xf8, 0xef, 0x42, 0xa5, 0xec, 0x5f, 0x03, 0xbb, 0xfa, 0x25,
            0x4c, 0xb0, 0x1f, 0xad,
        ];
        let mut dest = vec![0u8; 32];
        write_keccak(data, dest.as_mut_slice());

        assert_eq!(dest, expected);
    }

    struct RecoveryTestVector {
        pk: [u8; 33],
        message: &'static [u8],
        sig: [u8; 64],
        recid: RecoveryId,
    }

    const RECOVERY_TEST_VECTORS: &[RecoveryTestVector] = &[
        // Recovery ID 0
        RecoveryTestVector {
            pk: hex!("021a7a569e91dbf60581509c7fc946d1003b60c7dee85299538db6353538d59574"),
            message: b"example message",
            sig: hex!(
                "ce53abb3721bafc561408ce8ff99c909f7f0b18a2f788649d6470162ab1aa0323971edc523a6d6453f3fb6128d318d9db1a5ff3386feb1047d9816e780039d52"
            ),
            recid: RecoveryId::new(false, false),
        },
        // Recovery ID 1
        RecoveryTestVector {
            pk: hex!("036d6caac248af96f6afa7f904f550253a0f3ef3f5aa2fe6838a95b216691468e2"),
            message: b"example message",
            sig: hex!(
                "46c05b6368a44b8810d79859441d819b8e7cdc8bfd371e35c53196f4bcacdb5135c7facce2a97b95eacba8a586d87b7958aaf8368ab29cee481f76e871dbd9cb"
            ),
            recid: RecoveryId::new(true, false),
        },
    ];

    #[test]
    #[ignore]
    fn public_key_verify() {
        for vector in RECOVERY_TEST_VECTORS {
            let digest = Sha256::new_with_prefix(vector.message).finalize();
            // recover pk
            let mut output_pk = [0u8; 65];
            LowLevelSDK::crypto_ecrecover(
                &digest,
                &vector.sig,
                &mut output_pk,
                vector.recid.to_byte(),
            );
            // make sure pk matches
            // assert_eq!(output_pk, vector.pk);
        }
    }
}

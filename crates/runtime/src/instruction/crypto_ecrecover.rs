use crate::RuntimeContext;
use k256::{
    ecdsa::{RecoveryId, Signature, VerifyingKey},
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint,
    PublicKey,
};
use rwasm::{core::Trap, Caller};

pub struct CryptoEcrecover;

impl CryptoEcrecover {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        digest32_offset: u32,
        sig64_offset: u32,
        output65_offset: u32,
        rec_id: u32,
    ) -> Result<(), Trap> {
        let digest = caller.read_memory(digest32_offset, 32);
        let sig = caller.read_memory(sig64_offset, 64);
        caller.write_memory(output65_offset, &Self::fn_impl(digest, sig, rec_id));
        Ok(())
    }

    pub fn fn_impl(digest: &[u8], sig: &[u8], rec_id: u32) -> [u8; 65] {
        let sig = Signature::from_slice(sig).unwrap();
        let rec_id = RecoveryId::new(rec_id & 0b1 > 0, rec_id & 0b10 > 0);
        let pk = VerifyingKey::recover_from_prehash(digest, &sig, rec_id).unwrap();
        let pk_computed = EncodedPoint::from(&pk);
        let public_key = PublicKey::from_encoded_point(&pk_computed).unwrap();
        let pk_uncompressed = public_key.to_encoded_point(false);
        let mut result = [0u8; 65];
        result.copy_from_slice(pk_uncompressed.as_bytes());
        result
    }
}

#[cfg(test)]
mod secp256k1_tests {
    extern crate alloc;

    use crate::instruction::crypto_ecrecover::CryptoEcrecover;
    use hex_literal::hex;
    use sha2::{Digest, Sha256};

    struct RecoveryTestVector {
        pk: [u8; 33],
        msg: &'static [u8],
        sig: [u8; 64],
        rec_id: usize,
    }

    const RECOVERY_TEST_VECTORS: &[RecoveryTestVector] = &[
        // Recovery ID 0
        RecoveryTestVector {
            pk: hex!("021a7a569e91dbf60581509c7fc946d1003b60c7dee85299538db6353538d59574"),
            msg: b"example message",
            sig: hex!(
                "ce53abb3721bafc561408ce8ff99c909f7f0b18a2f788649d6470162ab1aa0323971edc523a6d6453f3fb6128d318d9db1a5ff3386feb1047d9816e780039d52"
            ),
            rec_id: 0,
        },
        // Recovery ID 1
        RecoveryTestVector {
            pk: hex!("036d6caac248af96f6afa7f904f550253a0f3ef3f5aa2fe6838a95b216691468e2"),
            msg: b"example message",
            sig: hex!(
                "46c05b6368a44b8810d79859441d819b8e7cdc8bfd371e35c53196f4bcacdb5135c7facce2a97b95eacba8a586d87b7958aaf8368ab29cee481f76e871dbd9cb"
            ),
            rec_id: 1,
        },
    ];

    #[test]
    fn public_key_recovery() {
        for vector in RECOVERY_TEST_VECTORS {
            let digest = Sha256::new_with_prefix(vector.msg).finalize();

            let mut params_vec: Vec<u8> = vec![];
            params_vec.extend(&digest);
            params_vec.extend(&vector.sig);
            params_vec.push(vector.rec_id as u8);
            params_vec.extend(&vector.pk);

            let _ = CryptoEcrecover::fn_impl(&digest, &vector.sig, vector.rec_id as u32);
            // &vector.pk;
            // assert_eq!(res, true);
        }
    }
}

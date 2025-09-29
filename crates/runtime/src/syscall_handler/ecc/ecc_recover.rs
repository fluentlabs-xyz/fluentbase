use super::ecc_config::{RecoverConfig, Secp256k1RecoverConfig};
use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm::{Store, TrapCode, Value};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message, SECP256K1,
};
use sp1_curves::CurveType;
use std::marker::PhantomData;

pub struct SyscallEccRecover<C: RecoverConfig> {
    _phantom: PhantomData<C>,
}

impl<C: RecoverConfig> SyscallEccRecover<C> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        match C::CURVE_TYPE {
            CurveType::Secp256k1 => Self::secp256k1_handler(caller, params, result),
            _ => Err(TrapCode::UnreachableCodeReached),
        }
    }

    pub fn fn_impl(digest: &B256, sig: &[u8], rec_id: u8) -> Option<Vec<u8>> {
        match C::CURVE_TYPE {
            CurveType::Secp256k1 => Self::secp256k1_recover_impl(digest, sig, rec_id),
            _ => None,
        }
    }

    fn secp256k1_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (digest_ptr, sig_ptr, output_ptr, rec_id) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as usize,
            params[2].i32().unwrap() as usize,
            params[3].i32().unwrap() as u32,
        );

        let mut digest = [0u8; Secp256k1RecoverConfig::DIGEST_SIZE];
        caller.memory_read(digest_ptr, &mut digest)?;

        let mut sig = [0u8; Secp256k1RecoverConfig::SIGNATURE_SIZE];
        caller.memory_read(sig_ptr, &mut sig)?;

        let public_key = Self::secp256k1_recover_impl(&B256::from(digest), &sig, rec_id as u8);
        match public_key {
            Some(public_key) => {
                caller.memory_write(output_ptr, &public_key)?;
                result[0] = Value::I32(0i32);
            }
            None => {
                result[0] = Value::I32(1i32);
            }
        };
        Ok(())
    }

    fn secp256k1_recover_impl(digest: &B256, sig: &[u8], rec_id: u8) -> Option<Vec<u8>> {
        // Ensure we have the correct signature size for Secp256k1
        if sig.len() != Secp256k1RecoverConfig::SIGNATURE_SIZE {
            return None;
        }

        let sig_array: [u8; Secp256k1RecoverConfig::SIGNATURE_SIZE] = sig.try_into().ok()?;
        let recid = match RecoveryId::try_from(rec_id as i32) {
            Ok(recid) => recid,
            Err(_) => return None,
        };

        let sig = match RecoverableSignature::from_compact(&sig_array, recid) {
            Ok(sig) => sig,
            Err(_) => return None,
        };
        let msg = Message::from_digest(digest.0);
        let public = match SECP256K1.recover_ecdsa(&msg, &sig) {
            Ok(public) => public,
            Err(_) => return None,
        };
        let uncompressed = public.serialize_uncompressed();
        Some(uncompressed.to_vec())
    }
}

#[cfg(test)]
mod secp256k1_tests {
    extern crate alloc;

    use super::{super::ecc_config::Secp256k1RecoverConfig, SyscallEccRecover};
    use fluentbase_types::B256;
    use hex_literal::hex;
    use k256::{elliptic_curve::sec1::ToEncodedPoint, PublicKey};
    use sha2::{Digest, Sha256};

    struct RecoveryTestVector {
        pk: [u8; 33],
        msg: &'static [u8],
        sig: [u8; 65],
        rec_id: u8,
    }

    const RECOVERY_TEST_VECTORS: &[RecoveryTestVector] = &[
        // Recovery ID 0
        RecoveryTestVector {
            pk: hex!("021a7a569e91dbf60581509c7fc946d1003b60c7dee85299538db6353538d59574"),
            msg: b"example message",
            sig: hex!(
                "04ce53abb3721bafc561408ce8ff99c909f7f0b18a2f788649d6470162ab1aa0323971edc523a6d6453f3fb6128d318d9db1a5ff3386feb1047d9816e780039d52"
            ),
            rec_id: 0,
        },
        // Recovery ID 1
        RecoveryTestVector {
            pk: hex!("036d6caac248af96f6afa7f904f550253a0f3ef3f5aa2fe6838a95b216691468e2"),
            msg: b"example message",
            sig: hex!(
                "0446c05b6368a44b8810d79859441d819b8e7cdc8bfd371e35c53196f4bcacdb5135c7facce2a97b95eacba8a586d87b7958aaf8368ab29cee481f76e871dbd9cb"
            ),
            rec_id: 1,
        },
    ];

    #[test]
    fn public_key_recovery() {
        for vector in RECOVERY_TEST_VECTORS {
            let digest = Sha256::new_with_prefix(vector.msg).finalize();

            let public_key = PublicKey::from_sec1_bytes(&vector.pk).unwrap();
            let pk_uncompressed = public_key.to_encoded_point(false);
            let expected_pk = pk_uncompressed.as_bytes();

            let result = SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(
                &B256::from_slice(&digest),
                &vector.sig[1..],
                vector.rec_id,
            )
            .unwrap();
            assert_eq!(result, expected_pk);
        }
    }

    #[test]
    fn public_key_recovery_failure() {
        let vector = RecoveryTestVector {
            pk: [0u8; 33],
            msg: b"example message",
            sig: [0u8; 65],
            rec_id: 1,
        };
        let digest = Sha256::new_with_prefix(vector.msg).finalize();

        let result = SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(
            &B256::from_slice(&digest),
            &vector.sig[1..],
            vector.rec_id,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_invalid_recovery_id() {
        let vector = &RECOVERY_TEST_VECTORS[0];
        let digest = Sha256::new_with_prefix(vector.msg).finalize();

        // Test with invalid recovery ID (should be 0-3 for secp256k1)
        let result = SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(
            &B256::from_slice(&digest),
            &vector.sig[1..],
            4, // Invalid recovery ID
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_invalid_signature_length() {
        let vector = &RECOVERY_TEST_VECTORS[0];
        let digest = Sha256::new_with_prefix(vector.msg).finalize();

        // Test with invalid signature length (should be 64 bytes)
        let invalid_sig = [0u8; 32];
        let result = SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(
            &B256::from_slice(&digest),
            &invalid_sig,
            vector.rec_id,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_invalid_signature_format() {
        let vector = &RECOVERY_TEST_VECTORS[0];
        let digest = Sha256::new_with_prefix(vector.msg).finalize();

        // Test with invalid signature format (all zeros)
        let invalid_sig = [0u8; 64];
        let result = SyscallEccRecover::<Secp256k1RecoverConfig>::fn_impl(
            &B256::from_slice(&digest),
            &invalid_sig,
            vector.rec_id,
        );
        assert!(result.is_none());
    }
}

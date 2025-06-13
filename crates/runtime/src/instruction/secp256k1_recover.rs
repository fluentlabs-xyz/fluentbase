use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm::{Caller, TrapCode};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message,
    SECP256K1,
};

pub struct SyscallSecp256k1Recover;

impl SyscallSecp256k1Recover {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let [digest32_ptr, sig64_ptr, output65_ptr, rec_id] = caller.stack_pop_n();
        let digest = caller.memory_read_fixed::<32>(digest32_ptr.as_usize())?;
        let sig = caller.memory_read_fixed::<64>(sig64_ptr.as_usize())?;
        let public_key = Self::fn_impl(&B256::from(digest), &sig, rec_id.as_u32() as u8);
        match public_key {
            Some(public_key) => {
                caller.memory_write(output65_ptr.as_usize(), &public_key)?;
                caller.stack_push(0);
            }
            None => {
                caller.stack_push(1);
            }
        };
        Ok(())
    }

    pub fn fn_impl(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        let recid = match RecoveryId::from_i32(rec_id as i32) {
            Ok(recid) => recid,
            Err(_) => return None,
        };
        let sig = match RecoverableSignature::from_compact(sig.as_slice(), recid) {
            Ok(sig) => sig,
            Err(_) => return None,
        };
        let msg = Message::from_digest(digest.0);
        let public = match SECP256K1.recover_ecdsa(&msg, &sig) {
            Ok(public) => public,
            Err(_) => return None,
        };
        Some(public.serialize_uncompressed())
    }
}

#[cfg(test)]
mod secp256k1_tests {
    extern crate alloc;

    use crate::instruction::secp256k1_recover::SyscallSecp256k1Recover;
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
            let mut pk = [0u8; 65];
            pk.copy_from_slice(pk_uncompressed.as_bytes());

            let result = SyscallSecp256k1Recover::fn_impl(
                &B256::from_slice(&digest),
                &vector.sig[1..].try_into().unwrap(),
                vector.rec_id,
            )
            .unwrap();
            assert_eq!(result, pk);
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

        let result = SyscallSecp256k1Recover::fn_impl(
            &B256::from_slice(&digest),
            &vector.sig[1..].try_into().unwrap(),
            vector.rec_id,
        );
        assert!(result.is_none());
    }
}

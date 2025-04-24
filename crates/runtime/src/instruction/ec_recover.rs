use crate::RuntimeContext;
use fluentbase_types::{ExitCode, B256};
use rwasm_executor::{Caller, RwasmError};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message,
    SECP256K1,
};

pub struct SyscallEcrecover;

impl SyscallEcrecover {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [digest32_ptr, sig64_ptr, output65_ptr, rec_id] = caller.stack_pop_n();
        let digest = caller.memory_read_fixed::<32>(digest32_ptr.as_usize())?;
        let sig = caller.memory_read_fixed::<64>(sig64_ptr.as_usize())?;
        let public_key = Self::fn_impl(&B256::from(digest), &sig, rec_id.as_u32() as u8)
            .map_err(|err| RwasmError::ExecutionHalted(err.into_i32()))?;
        caller.memory_write(output65_ptr.as_usize(), &public_key)?;
        Ok(())
    }

    pub fn fn_impl(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Result<[u8; 65], ExitCode> {
        let recid = RecoveryId::from_i32(rec_id as i32).expect("recovery ID is valid");
        let sig = RecoverableSignature::from_compact(sig.as_slice(), recid)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let msg = Message::from_digest(digest.0);
        let public = SECP256K1
            .recover_ecdsa(&msg, &sig)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        Ok(public.serialize_uncompressed())
    }
}

#[cfg(test)]
mod secp256k1_tests {
    extern crate alloc;

    use crate::instruction::ec_recover::SyscallEcrecover;
    use fluentbase_types::B256;
    use hex_literal::hex;
    use k256::{elliptic_curve::sec1::ToEncodedPoint, PublicKey};
    use sha2::{Digest, Sha256};

    struct RecoveryTestVector {
        pk: [u8; 33],
        msg: &'static [u8],
        sig: [u8; 65],
        rec_id: usize,
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

            let mut params_vec: Vec<u8> = vec![];
            params_vec.extend(&digest);
            params_vec.extend(&vector.sig);
            params_vec.push(vector.rec_id as u8);
            params_vec.extend(&vector.pk);

            let public_key = PublicKey::from_sec1_bytes(&vector.pk).unwrap();
            let pk_uncompressed = public_key.to_encoded_point(false);
            let mut pk = [0u8; 65];
            pk.copy_from_slice(pk_uncompressed.as_bytes());

            let result = SyscallEcrecover::fn_impl(
                &B256::from_slice(&digest),
                &vector.sig[1..].try_into().unwrap(),
                vector.rec_id as u8,
            )
            .unwrap();
            assert_eq!(result, pk);
        }
    }
}

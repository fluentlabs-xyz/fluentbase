use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm_executor::{Caller, RwasmError};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message,
    SECP256K1,
};

pub struct SyscallSecp256k1Recover;

impl SyscallSecp256k1Recover {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
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
        let recid = RecoveryId::from_i32(rec_id as i32).expect("recovery ID is valid");
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

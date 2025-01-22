use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError, TrapCode};
use fluentbase_types::Bytes;

pub struct SyscallPreimageCopy;

impl SyscallPreimageCopy {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let (hash32_ptr, preimage_ptr) = caller.stack_pop2_as::<u32>();
        let hash = caller.read_memory(hash32_ptr, 32)?.to_vec();
        let preimage = Self::fn_impl(caller.data_mut(), &hash)?;
        caller.write_memory(preimage_ptr, preimage.as_ref())?;
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<Bytes, RwasmError> {
        Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached))
    }
}

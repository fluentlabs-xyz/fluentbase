use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError, TrapCode};

pub struct SyscallPreimageSize;

impl SyscallPreimageSize {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let hash32_offset: u32 = caller.stack_pop_as();
        let hash = caller.read_memory(hash32_offset, 32)?.to_vec();
        let preimage_size = Self::fn_impl(caller.data_mut(), &hash)?;
        caller.stack_push(preimage_size);
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<u32, RwasmError> {
        Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached))
    }
}

use crate::RuntimeContext;
use fluentbase_types::Bytes;
use rwasm_executor::{Caller, RwasmError, TrapCode};

pub struct SyscallPreimageCopy;

impl SyscallPreimageCopy {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let (hash32_ptr, preimage_ptr) = caller.stack_pop2_as::<u32>();
        let hash = caller.memory_read_fixed::<32>(hash32_ptr as usize)?;
        let preimage = Self::fn_impl(caller.context_mut(), &hash)?;
        caller.memory_write(preimage_ptr as usize, preimage.as_ref())?;
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<Bytes, RwasmError> {
        Err(RwasmError::TrapCode(TrapCode::UnreachableCodeReached))
    }
}

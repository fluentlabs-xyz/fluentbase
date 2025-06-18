use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallPreimageSize;

impl SyscallPreimageSize {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let hash32_offset: u32 = caller.stack_pop_as();
        let hash = caller
            .memory_read_fixed::<32>(hash32_offset as usize)?
            .to_vec();
        let preimage_size = Self::fn_impl(caller.context_mut(), &hash)?;
        caller.stack_push(preimage_size);
        Ok(())
    }

    pub fn fn_impl(_ctx: &RuntimeContext, _hash: &[u8]) -> Result<u32, TrapCode> {
        Err(TrapCode::UnreachableCodeReached)
    }
}

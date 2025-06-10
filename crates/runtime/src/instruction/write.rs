use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallWrite;

impl SyscallWrite {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let (offset, length) = caller.stack_pop2_as::<u32>();
        let data = caller.memory_read_vec(offset as usize, length as usize)?;
        Self::fn_impl(caller.context_mut(), &data);
        Ok(())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, data: &[u8]) {
        ctx.execution_result.output.extend_from_slice(data);
    }
}

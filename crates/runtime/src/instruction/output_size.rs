use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallOutputSize;

impl SyscallOutputSize {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let output_size = Self::fn_impl(caller.context());
        caller.stack_push(output_size);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.execution_result.return_data.len() as u32
    }
}

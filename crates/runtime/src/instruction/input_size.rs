use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallInputSize;

impl SyscallInputSize {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let input_size = Self::fn_impl(caller.context());
        caller.stack_push(input_size);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.input_size()
    }
}

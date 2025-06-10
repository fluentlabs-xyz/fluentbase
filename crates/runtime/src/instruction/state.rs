use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};

pub struct SyscallState;

impl SyscallState {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let state = Self::fn_impl(caller.context());
        caller.stack_push(state);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.state
    }
}

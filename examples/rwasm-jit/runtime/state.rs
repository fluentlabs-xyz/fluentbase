use crate::RuntimeContext;
use rwasm::{core::Trap, Caller};

pub struct SyscallState;

impl SyscallState {
    pub fn fn_handler(caller: Caller<'_, RuntimeContext>) -> Result<u32, Trap> {
        Ok(Self::fn_impl(caller.data()))
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.state
    }
}

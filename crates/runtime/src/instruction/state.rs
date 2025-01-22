use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};

pub struct SyscallState;

impl SyscallState {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let state = Self::fn_impl(caller.data());
        caller.stack_push(state);
        Ok(())
    }

    pub fn fn_impl(ctx: &RuntimeContext) -> u32 {
        ctx.state
    }
}

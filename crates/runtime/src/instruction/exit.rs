use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::ExitCode;

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let exit_code: i32 = caller.stack_pop_as();
        let exit_code = Self::fn_impl(caller.context_mut(), exit_code).unwrap_err();
        Err(RwasmError::ExecutionHalted(exit_code.into_i32()))
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, exit_code: i32) -> Result<(), ExitCode> {
        if exit_code > 0 {
            return Err(ExitCode::NonNegativeExitCode);
        }
        ctx.execution_result.exit_code = exit_code;
        Err(ExitCode::Ok)
    }
}

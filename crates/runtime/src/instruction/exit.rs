use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let exit_code: i32 = caller.stack_pop_as();
        Self::fn_impl(caller.context_mut(), exit_code).unwrap_err();
        Err(TrapCode::ExecutionHalted)
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, exit_code: i32) -> Result<(), ExitCode> {
        if exit_code > 0 {
            return Err(ExitCode::NonNegativeExitCode);
        }
        ctx.execution_result.exit_code = exit_code;
        Err(ExitCode::Ok)
    }
}

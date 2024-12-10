use crate::RuntimeContext;
use fluentbase_sdk::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, exit_code: i32) -> Result<(), Trap> {
        let exit_code = Self::fn_impl(caller.data_mut(), exit_code).unwrap_err();
        Err(exit_code.into_trap())
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, exit_code: i32) -> Result<(), ExitCode> {
        if exit_code > 0 {
            return Err(ExitCode::NonNegativeExitCode);
        }
        ctx.execution_result.exit_code = exit_code;
        Err(ExitCode::ExecutionHalted)
    }
}

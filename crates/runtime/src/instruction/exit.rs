use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let exit_code = params[0].i32().unwrap();
        caller.context_mut(|ctx| Self::fn_impl(ctx, exit_code).unwrap_err());
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

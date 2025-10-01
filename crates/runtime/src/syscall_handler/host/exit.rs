use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, Value};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let exit_code = params[0].i32().unwrap();
        caller.context_mut(|ctx| Self::fn_impl(ctx, ExitCode::from(exit_code)).unwrap_err());
        Err(TrapCode::ExecutionHalted)
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, exit_code: ExitCode) -> Result<(), ExitCode> {
        ctx.execution_result.exit_code = exit_code.into_i32();
        Err(ExitCode::Ok)
    }
}

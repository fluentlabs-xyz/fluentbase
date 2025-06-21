use crate::RuntimeContext;
use core::cell::RefMut;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode, Value};

pub struct SyscallExit;

impl SyscallExit {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let exit_code = params[0].i32().unwrap();
        Self::fn_impl(caller.context_mut(), exit_code).unwrap_err();
        Err(TrapCode::ExecutionHalted)
    }

    pub fn fn_impl(mut ctx: RefMut<RuntimeContext>, exit_code: i32) -> Result<(), ExitCode> {
        if exit_code > 0 {
            return Err(ExitCode::NonNegativeExitCode);
        }
        ctx.execution_result.exit_code = exit_code;
        Err(ExitCode::Ok)
    }
}

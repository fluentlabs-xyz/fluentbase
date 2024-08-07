use crate::{Runtime, RuntimeContext};
use core::mem::take;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        call_id: u32,
        exit_code: i32,
    ) -> Result<i32, Trap> {
        let (_fuel_remaining, exit_code) = Self::fn_impl(caller.data_mut(), call_id, exit_code);
        Ok(exit_code)
    }

    pub fn fn_impl(ctx: &mut RuntimeContext, call_id: u32, exit_code: i32) -> (u64, i32) {
        // only root can use resume function
        if ctx.depth > 0 {
            return (0, ExitCode::RootCallOnly.into_i32());
        }

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");
        let context = take(&mut ctx.context);

        // move jzkt and context into recovered execution state
        recoverable_runtime.runtime.store_mut().data_mut().jzkt = Some(jzkt);
        recoverable_runtime.runtime.store_mut().data_mut().context = context;

        let execution_result = recoverable_runtime.runtime.resume(exit_code);

        // return jzkt context back
        ctx.jzkt = take(&mut recoverable_runtime.runtime.store.data_mut().jzkt);
        ctx.context = take(&mut recoverable_runtime.runtime.store.data_mut().context);

        let state = recoverable_runtime.state();

        // make sure there is no return overflow
        if state.return_len > 0 && execution_result.output.len() > state.return_len as usize {
            return (0, ExitCode::OutputOverflow.into_i32());
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        // increase total fuel consumed and remember return data
        ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        ctx.execution_result.return_data = execution_result.output.clone();

        (
            state.delegated_execution.fuel as u64 - execution_result.fuel_consumed,
            execution_result.exit_code,
        )
    }
}

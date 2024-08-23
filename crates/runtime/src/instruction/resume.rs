use crate::{Runtime, RuntimeContext};
use core::mem::take;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        call_id: u32,
        return_data_ptr: u32,
        return_data_len: u32,
        exit_code: i32,
        fuel_used: u64,
    ) -> Result<i32, Trap> {
        let return_data = caller
            .read_memory(return_data_ptr, return_data_len)?
            .to_vec();
        let exit_code = Self::fn_impl(
            caller.data_mut(),
            call_id,
            return_data,
            exit_code,
            fuel_used,
        );
        Ok(exit_code)
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        call_id: u32,
        return_data: Vec<u8>,
        exit_code: i32,
        fuel_used: u64,
    ) -> i32 {
        // only root can use resume function
        if ctx.call_depth > 0 {
            return ExitCode::RootCallOnly.into_i32();
        }

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        // during the résumé we must clear output, otherwise collision might happen
        recoverable_runtime
            .runtime
            .store_mut()
            .data_mut()
            .clear_output();

        // charge fuel
        if !recoverable_runtime
            .runtime
            .store_mut()
            .data_mut()
            .fuel
            .charge(fuel_used)
        {
            return ExitCode::OutOfGas.into_i32();
        }

        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");
        let context = take(&mut ctx.context);

        // move jzkt and context into recovered execution state
        recoverable_runtime.runtime.store_mut().data_mut().jzkt = Some(jzkt);
        recoverable_runtime.runtime.store_mut().data_mut().context = context;

        // copy return data into return data
        let return_data_mut = recoverable_runtime
            .runtime
            .store_mut()
            .data_mut()
            .return_data_mut();
        return_data_mut.clear();
        return_data_mut.extend(&return_data);

        let mut execution_result = recoverable_runtime.runtime.resume(exit_code);

        // return jzkt context back
        ctx.jzkt = take(&mut recoverable_runtime.runtime.store.data_mut().jzkt);
        ctx.context = take(&mut recoverable_runtime.runtime.store.data_mut().context);

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = recoverable_runtime.runtime.remember_runtime() as i32;
        } else {
            // refund unspent fuel back
            ctx.fuel.charge(execution_result.fuel_consumed);
            // increase total fuel consumed and remember return data
            ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        execution_result.exit_code
    }
}

use crate::{Runtime, RuntimeContext};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    ExitCode,
};
#[cfg(feature = "wasmtime")]
use num::ToPrimitive;
use rwasm::{Caller, TrapCode};

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let [call_id, return_data_ptr, return_data_len, exit_code, fuel16_ptr] =
            caller.stack_pop_n();
        let return_data = caller
            .memory_read_vec(return_data_ptr.as_usize(), return_data_len.as_usize())?
            .to_vec();
        let fuel16_ptr = fuel16_ptr.as_usize();
        let (fuel_consumed, fuel_refunded) = if fuel16_ptr > 0 {
            let mut fuel_buffer = [0u8; 16];
            caller.memory_read(fuel16_ptr, &mut fuel_buffer)?;
            let fuel_consumed = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
            let fuel_refunded = LittleEndian::read_i64(&fuel_buffer[8..]);
            (fuel_consumed, fuel_refunded)
        } else {
            (0, 0)
        };
        let (fuel_consumed, fuel_refunded, exit_code) = Self::fn_impl(
            caller.context_mut(),
            call_id.as_u32(),
            return_data,
            exit_code.as_i32(),
            fuel_consumed,
            fuel_refunded,
            fuel16_ptr as u32,
        );
        if fuel16_ptr > 0 {
            caller.memory_write(fuel16_ptr, &fuel_consumed.to_le_bytes())?;
            caller.memory_write(fuel16_ptr + 8, &fuel_refunded.to_le_bytes())?;
        }
        caller.stack_push(exit_code);
        Ok(())
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        call_id: u32,
        return_data: Vec<u8>,
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        fuel16_ptr: u32,
    ) -> (u64, i64, i32) {
        // only root can use resume function
        if ctx.call_depth > 0 {
            return (0, 0, ExitCode::RootCallOnly.into_i32());
        }

        #[cfg(feature = "wasmtime")]
        if let Some((fuel_consumed, fuel_refunded, exit_code, output)) = crate::wasmtime::try_resume(
            call_id.to_i32().unwrap(),
            return_data.clone(),
            exit_code,
            fuel_consumed,
            fuel_refunded,
            fuel16_ptr,
        ) {
            ctx.execution_result.return_data = output;
            return (fuel_consumed, fuel_refunded, exit_code);
        }

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        // during the résumé we must-clear output, otherwise collision might happen
        recoverable_runtime.context_mut().clear_output();

        // we can charge fuel only if fuel is not disabled,
        // when fuel is disabled we only pass consumed fuel amount into the contract back,
        // and it can decide on charging
        if !ctx.disable_fuel && fuel_consumed > 0 {
            let store = &mut recoverable_runtime.store;
            // charge fuel that was spent during the interruption
            // to make sure our fuel calculations are aligned
            if let Err(_) = store.try_consume_fuel(fuel_consumed) {
                return (0, 0, ExitCode::OutOfFuel.into_i32());
            }
        }

        // copy return data into return data
        let return_data_mut = recoverable_runtime.store.context_mut().return_data_mut();
        return_data_mut.clear();
        return_data_mut.extend(&return_data);

        let mut execution_result =
            recoverable_runtime.resume(fuel16_ptr, fuel_consumed, fuel_refunded, exit_code);

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = recoverable_runtime.remember_runtime(ctx);
        }

        ctx.execution_result.return_data = execution_result.output.clone();

        (
            execution_result.fuel_consumed,
            execution_result.fuel_refunded,
            execution_result.exit_code,
        )
    }
}

use crate::{Runtime, RuntimeContext};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    ExitCode,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (call_id, return_data_ptr, return_data_len, exit_code, fuel16_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as usize,
            params[2].i32().unwrap() as usize,
            params[3].i32().unwrap(),
            params[4].i32().unwrap() as usize,
        );
        let mut return_data = vec![0u8; return_data_len];
        caller.memory_read(return_data_ptr, &mut return_data)?;
        let (fuel_consumed, fuel_refunded) = if fuel16_ptr > 0 {
            let mut fuel_buffer = [0u8; 16];
            caller.memory_read(fuel16_ptr, &mut fuel_buffer)?;
            let fuel_consumed = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
            let fuel_refunded = LittleEndian::read_i64(&fuel_buffer[8..]);
            (fuel_consumed, fuel_refunded)
        } else {
            (0, 0)
        };
        let (fuel_consumed, fuel_refunded, exit_code) = caller.context_mut(|ctx| {
            Self::fn_impl(
                ctx,
                call_id,
                &return_data,
                exit_code,
                fuel_consumed,
                fuel_refunded,
                fuel16_ptr as u32,
            )
        });
        if fuel16_ptr > 0 {
            caller.memory_write(fuel16_ptr, &fuel_consumed.to_le_bytes())?;
            caller.memory_write(fuel16_ptr + 8, &fuel_refunded.to_le_bytes())?;
        }
        result[0] = Value::I32(exit_code);
        Ok(())
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        fuel16_ptr: u32,
    ) -> (u64, i64, i32) {
        // only root can use resume function
        if ctx.call_depth > 0 {
            return (0, 0, ExitCode::RootCallOnly.into_i32());
        }

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        // during the résumé we must clear output, otherwise collision might happen
        recoverable_runtime
            .store
            .context_mut(|ctx| ctx.clear_output());

        // we can charge fuel only if fuel is not disabled,
        // when fuel is disabled,
        // we only pass consumed fuel amount into the contract back,
        // and it can decide on charging
        if !ctx.disable_fuel && fuel_consumed > 0 {
            // charge fuel that was spent during the interruption
            // to make sure our fuel calculations are aligned
            if let Err(_) = recoverable_runtime.store.try_consume_fuel(fuel_consumed) {
                return (0, 0, ExitCode::OutOfFuel.into_i32());
            }
        }

        // copy return data into return data
        recoverable_runtime.store.context_mut(|ctx| {
            ctx.return_data_mut().clear();
            ctx.return_data_mut().extend(return_data);
        });

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

use crate::{Runtime, RuntimeContext};
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::ExitCode;

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(_caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        todo!("make sure this function works in the root mode");
        // let [call_id, return_data_ptr, return_data_len, exit_code, fuel16_ptr] =
        //     caller.stack_pop_n();
        // let return_data = caller
        //     .memory_read_vec(return_data_ptr.as_usize(), return_data_len.as_usize())?
        //     .to_vec();
        // let fuel16_ptr = fuel16_ptr.as_usize();
        // let (fuel_consumed, fuel_refunded) = if fuel16_ptr > 0 {
        //     let mut fuel_buffer = [0u8; 16];
        //     caller.memory_read(fuel16_ptr, &mut fuel_buffer)?;
        //     let fuel_consumed = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
        //     let fuel_refunded = LittleEndian::read_i64(&fuel_buffer[8..]);
        //     (fuel_consumed, fuel_refunded)
        // } else {
        //     (0, 0)
        // };
        // let (fuel_consumed, fuel_refunded, exit_code) = Self::fn_impl(
        //     caller.data_mut(),
        //     call_id.as_u32(),
        //     return_data,
        //     exit_code.as_i32(),
        //     fuel_consumed,
        //     fuel_refunded,
        // );
        // caller.stack_push(exit_code);
        // Ok(())
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

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        // during the résumé we must-clear output, otherwise collision might happen
        recoverable_runtime.context_mut().clear_output();

        // we can charge fuel only if fuel is not disabled,
        // when fuel is disabled we only pass consumed fuel amount into the contract back,
        // and it can decide on charging
        if !ctx.disable_fuel && fuel_consumed > 0 {
            let store = recoverable_runtime.executor.store_mut();
            // charge fuel that was spent during the interruption
            // to make sure our fuel calculations are aligned
            if let Err(_) = store.try_consume_fuel(fuel_consumed) {
                return (0, 0, ExitCode::OutOfFuel.into_i32());
            }
        }

        // copy return data into return data
        let return_data_mut = recoverable_runtime
            .executor
            .store_mut()
            .context_mut()
            .return_data_mut();
        return_data_mut.clear();
        return_data_mut.extend(&return_data);

        // let _skip_trace_logs = recoverable_runtime
        //     .runtime
        //     .store
        //     .tracer()
        //     .unwrap()
        //     .logs
        //     .len();

        let mut execution_result =
            recoverable_runtime.resume(fuel16_ptr, fuel_consumed, fuel_refunded, exit_code);

        // println!("\n\nRESUME, interrupted: {}", execution_result.interrupted);
        // println!(
        //     "exit_code: {} ({})",
        //     execution_result.exit_code,
        //     ExitCode::from(execution_result.exit_code)
        // );
        // println!(
        //     "output: 0x{} ({})",
        //     hex::encode(&execution_result.output),
        //     std::str::from_utf8(&execution_result.output).unwrap_or("can't decode utf-8")
        // );
        // println!("fuel consumed: {}", execution_result.fuel_consumed);
        // let logs = &recoverable_runtime.runtime.store().tracer().unwrap().logs;
        // println!("execution trace ({} steps):", logs.len());
        // for log in logs.iter().skip(skip_trace_logs).rev().take(100).rev() {
        //     use rwasm::rwasm::instruction::InstructionExtra;
        //     if let Some(value) = log.opcode.aux_value() {
        //         println!(
        //             " - pc={} opcode={}({}) gas={} stack={:?}",
        //             log.program_counter,
        //             log.opcode,
        //             value,
        //             log.consumed_fuel,
        //             log.stack
        //                 .iter()
        //                 .map(|v| v.to_string())
        //                 .rev()
        //                 .take(3)
        //                 .rev()
        //                 .collect::<Vec<_>>(),
        //         );
        //     } else {
        //         println!(
        //             " - pc={} opcode={} gas={} stack={:?}",
        //             log.program_counter,
        //             log.opcode,
        //             log.consumed_fuel,
        //             log.stack
        //                 .iter()
        //                 .map(|v| v.to_string())
        //                 .rev()
        //                 .take(3)
        //                 .rev()
        //                 .collect::<Vec<_>>()
        //         );
        //     }
        // }

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = recoverable_runtime.remember_runtime(ctx);
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        (
            execution_result.fuel_consumed,
            0,
            execution_result.exit_code,
        )
    }
}

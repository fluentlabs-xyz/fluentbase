use crate::{Runtime, RuntimeContext};
use core::mem::take;
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    ExitCode,
};
use rwasm::{core::Trap, errors::FuelError, Caller};

pub struct SyscallResume;

impl SyscallResume {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        call_id: u32,
        return_data_ptr: u32,
        return_data_len: u32,
        exit_code: i32,
        fuel_ptr: u32,
    ) -> Result<i32, Trap> {
        let return_data = caller
            .read_memory(return_data_ptr, return_data_len)?
            .to_vec();
        let fuel_spent = LittleEndian::read_u64(caller.read_memory(fuel_ptr, 8)?);
        let (fuel_consumed, exit_code) = Self::fn_impl(
            caller.data_mut(),
            call_id,
            return_data,
            exit_code,
            fuel_spent,
        );
        if let Err(err) = caller.consume_fuel(fuel_consumed) {
            match err {
                FuelError::FuelMeteringDisabled => {}
                FuelError::OutOfFuel => return Err(ExitCode::OutOfGas.into_trap()),
            }
        }
        let mut fuel_buffer = [0u8; 8];
        LittleEndian::write_u64(&mut fuel_buffer, fuel_consumed);
        caller.write_memory(fuel_ptr, &fuel_buffer)?;
        Ok(exit_code)
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        call_id: u32,
        return_data: Vec<u8>,
        exit_code: i32,
        fuel_used: u64,
    ) -> (u64, i32) {
        // only root can use resume function
        if ctx.call_depth > 0 {
            return (0, ExitCode::RootCallOnly.into_i32());
        }

        let mut recoverable_runtime = Runtime::recover_runtime(call_id);

        // during the résumé we must clear output, otherwise collision might happen
        recoverable_runtime
            .runtime
            .store_mut()
            .data_mut()
            .clear_output();

        let fuel_consumed_before_call = recoverable_runtime
            .runtime
            .store()
            .fuel_consumed()
            .unwrap_or_default();

        // charge fuel
        if let Err(err) = recoverable_runtime
            .runtime
            .store_mut()
            .consume_fuel(fuel_used)
        {
            match err {
                FuelError::FuelMeteringDisabled => {}
                FuelError::OutOfFuel => return (0, ExitCode::OutOfGas.into_i32()),
            }
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

        let skip_trace_logs = recoverable_runtime
            .runtime
            .store
            .tracer()
            .unwrap()
            .logs
            .len();

        let mut execution_result = recoverable_runtime
            .runtime
            .resume(exit_code, fuel_consumed_before_call);

        // return jzkt context back
        ctx.jzkt = take(&mut recoverable_runtime.runtime.store.data_mut().jzkt);
        ctx.context = take(&mut recoverable_runtime.runtime.store.data_mut().context);

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
            execution_result.exit_code = recoverable_runtime.runtime.remember_runtime() as i32;
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        (execution_result.fuel_consumed, execution_result.exit_code)
    }
}

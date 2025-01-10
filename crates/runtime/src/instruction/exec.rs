use crate::{
    types::{NonePreimageResolver, PreimageResolver},
    Runtime,
    RuntimeContext,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    Bytes,
    ExitCode,
    SyscallInvocationParams,
    B256,
};
use rwasm::{
    core::{HostError, Trap},
    errors::FuelError,
    Caller,
};
use std::fmt::{Debug, Display, Formatter};

pub struct SyscallExec;

#[derive(Default, Clone)]
pub struct SysExecResumable {
    /// List of delayed invocation params, like exec params (address, code hash, etc.)
    pub params: SyscallInvocationParams,
    /// A depth level of the current call, for root it's always zero
    pub is_root: bool,
    /// Return consumed fuel result into this pointer (u64 value)
    pub fuel_ptr: u32,
}

pub const CALL_STACK_LIMIT: u32 = 1024;

impl Debug for SysExecResumable {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl Display for SysExecResumable {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl HostError for SysExecResumable {}

impl SyscallExec {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel_ptr: u32,
        state: u32,
    ) -> Result<i32, Trap> {
        // make sure we have enough fuel for this call
        let fuel_limit = if fuel_ptr > 0 {
            let mut fuel_limit = LittleEndian::read_u64(caller.read_memory(fuel_ptr, 8)?);
            if fuel_limit == 0 {
                fuel_limit = caller.fuel_remaining().unwrap_or_default();
            }
            if fuel_limit > caller.fuel_remaining().unwrap_or(fuel_limit) {
                return Err(ExitCode::OutOfGas.into_trap());
            }
            fuel_limit
        } else {
            caller.fuel_remaining().unwrap_or_default()
        };
        // return resumable error
        Err(SysExecResumable {
            params: SyscallInvocationParams {
                code_hash: B256::from_slice(caller.read_memory(hash32_ptr, 32)?),
                input: Bytes::copy_from_slice(caller.read_memory(input_ptr, input_len)?),
                fuel_limit,
                state,
            },
            is_root: caller.data().call_depth == 0,
            fuel_ptr,
        }
        .into())
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        context: &SysExecResumable,
    ) -> Result<i32, Trap> {
        let (fuel_consumed, exit_code) = Self::fn_impl(
            caller.data_mut(),
            &context.params.code_hash,
            context.params.input.as_ref(),
            context.params.fuel_limit,
            context.params.state,
        );
        if let Err(err) = caller.consume_fuel(fuel_consumed) {
            match err {
                FuelError::FuelMeteringDisabled => {}
                FuelError::OutOfFuel => return Err(ExitCode::OutOfGas.into_trap()),
            }
        }
        if context.fuel_ptr > 0 {
            let mut fuel_buffer = [0u8; 8];
            LittleEndian::write_u64(&mut fuel_buffer, fuel_consumed);
            caller.write_memory(context.fuel_ptr, &fuel_buffer)?;
        }
        Ok(exit_code)
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        code_hash: &B256,
        input: &[u8],
        fuel_limit: u64,
        state: u32,
    ) -> (u64, i32) {
        Self::fn_impl_ex(
            ctx,
            code_hash,
            input,
            fuel_limit,
            state,
            &NonePreimageResolver,
        )
    }

    pub fn fn_impl_ex<PR: PreimageResolver>(
        ctx: &mut RuntimeContext,
        code_hash: &B256,
        input: &[u8],
        fuel_limit: u64,
        state: u32,
        preimage_resolver: &PR,
    ) -> (u64, i32) {
        // check call depth overflow
        if ctx.call_depth >= CALL_STACK_LIMIT {
            return (fuel_limit, ExitCode::CallDepthOverflow.into_i32());
        }

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(*code_hash)
            .with_input(input.to_vec())
            .with_fuel_limit(fuel_limit)
            // .with_preimage_resolver(ctx.preimage_resolver.clone())
            .with_state(state)
            .with_depth(ctx.call_depth + 1)
            .with_disable_fuel(ctx.disable_fuel);
        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call_with_preimage_resolver(preimage_resolver);

        // println!("\n\nEXEC, interrupted: {}", execution_result.interrupted);
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
        // let logs = &runtime.store().tracer().unwrap().logs;
        // println!("execution trace ({} steps):", logs.len());
        // for log in logs.iter().rev().take(100).rev() {
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
        //                 .collect::<Vec<_>>(),
        //         );
        //     }
        // }

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = runtime.remember_runtime() as i32;
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        (execution_result.fuel_consumed, execution_result.exit_code)
    }
}

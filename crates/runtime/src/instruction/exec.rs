use crate::{
    types::{NonePreimageResolver, PreimageResolver},
    Runtime,
    RuntimeContext,
};
use fluentbase_rwasm::{Caller, HostError, RwasmError, TrapCode};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    Bytes,
    ExitCode,
    SyscallInvocationParams,
    B256,
    FUEL_DENOM_RATE,
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
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [hash32_ptr, input_ptr, input_len, fuel8_ptr, state] = caller.stack_pop_n();
        // make sure we have enough fuel for this call
        let fuel_limit = if fuel8_ptr.as_u32() > 0 {
            let mut fuel_limit =
                LittleEndian::read_u64(&caller.memory_read_fixed::<8>(fuel8_ptr.as_usize())?);
            if fuel_limit == 0 {
                fuel_limit = caller.store().remaining_fuel().unwrap_or_default();
            }
            if fuel_limit > caller.store().remaining_fuel().unwrap_or(u64::MAX) {
                return Err(RwasmError::TrapCode(TrapCode::OutOfFuel));
            }
            fuel_limit
        } else {
            caller.store().remaining_fuel().unwrap_or(u64::MAX)
        };
        // calculate gas limit as lower bounded (for charging gas, we use upper bound rounding,
        // that is why we need to do lower bound rounding for gas limit)
        let gas_limit = fuel_limit / FUEL_DENOM_RATE;
        // return resumable error
        Err(RwasmError::HostInterruption(Box::new(SysExecResumable {
            params: SyscallInvocationParams {
                code_hash: B256::from(caller.memory_read_fixed::<32>(hash32_ptr.as_usize())?),
                input: Bytes::copy_from_slice(
                    &caller.memory_read_vec(input_ptr.as_usize(), input_len.as_usize())?,
                ),
                gas_limit,
                state: state.as_u32(),
            },
            is_root: caller.store().context().call_depth == 0,
            fuel_ptr: fuel8_ptr.as_u32(),
        })))
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        context: &SysExecResumable,
    ) -> Result<i32, RwasmError> {
        let (fuel_consumed, exit_code) = Self::fn_impl(
            caller.store_mut().context_mut(),
            &context.params.code_hash,
            context.params.input.as_ref(),
            context.params.gas_limit,
            context.params.state,
        );
        caller.store_mut().try_consume_fuel(fuel_consumed)?;
        if context.fuel_ptr > 0 {
            let mut fuel_buffer = [0u8; 8];
            LittleEndian::write_u64(&mut fuel_buffer, fuel_consumed);
            caller.memory_write(context.fuel_ptr as usize, &fuel_buffer)?;
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

        // let preimage = preimage_resolver.preimage(&code_hash.0).unwrap_or_default();
        // let mut syscall_handler = SimpleCallHandler::default();
        // syscall_handler.input = input.to_vec();
        // syscall_handler.state = state;
        // let mut executor = RwasmExecutor::parse(
        //     preimage.as_ref(),
        //     Some(&mut syscall_handler),
        //     Some(fuel_limit),
        // )
        // .unwrap();
        // let exit_code = executor.run().unwrap();
        // ctx.execution_result.return_data = syscall_handler.output.clone();
        // return (0, exit_code);

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(*code_hash)
            .with_input(Bytes::copy_from_slice(input))
            .with_fuel_limit(fuel_limit)
            .with_state(state)
            .with_depth(ctx.call_depth + 1)
            .with_disable_fuel(ctx.disable_fuel);
        let mut runtime = Runtime::new(ctx2, preimage_resolver);
        let mut execution_result = runtime.call();

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

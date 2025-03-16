use crate::{Runtime, RuntimeContext};
use fluentbase_rwasm::{Caller, HostError, RwasmError, TrapCode};
use fluentbase_types::{
    BytecodeOrHash,
    Bytes,
    ExitCode,
    SyscallInvocationParams,
    B256,
    CALL_STACK_LIMIT,
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
}

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
        let [hash32_ptr, input_ptr, input_len, fuel_limit, state] = caller.stack_pop_n();
        // make sure we have enough fuel for this call
        let fuel_limit = fuel_limit.as_u64();
        let fuel_limit = if fuel_limit > 0 {
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
                input: Bytes::from(
                    caller.memory_read_vec(input_ptr.as_usize(), input_len.as_usize())?,
                ),
                gas_limit,
                state: state.as_u32(),
            },
            is_root: caller.store().context().call_depth == 0,
        })))
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        context: &SysExecResumable,
    ) -> (u32, i32) {
        let fuel_limit = context.params.gas_limit * FUEL_DENOM_RATE;
        let (fuel_consumed, exit_code) = Self::fn_impl(
            caller.store_mut().context_mut(),
            context.params.code_hash,
            context.params.input.as_ref(),
            fuel_limit,
            context.params.state,
        );
        (fuel_consumed as u32, exit_code)
    }

    pub fn fn_impl<I: Into<BytecodeOrHash>>(
        ctx: &mut RuntimeContext,
        code_hash: I,
        input: &[u8],
        fuel_limit: u64,
        state: u32,
    ) -> (u64, i32) {
        // check call depth overflow
        if ctx.call_depth >= CALL_STACK_LIMIT {
            return (fuel_limit, ExitCode::CallDepthOverflow.into_i32());
        }

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new(code_hash)
            .with_input(Bytes::copy_from_slice(input))
            .with_fuel_limit(fuel_limit)
            .with_state(state)
            .with_call_depth(ctx.call_depth + 1)
            .with_disable_fuel(ctx.disable_fuel);
        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call();

        // let trace = runtime.store().tracer().unwrap().logs.len();
        // println!("execution trace ({} steps):", trace);

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
            execution_result.exit_code = runtime.remember_runtime(ctx);
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        (execution_result.fuel_consumed, execution_result.exit_code)
    }
}

use crate::{Runtime, RuntimeContext};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    BytecodeOrHash,
    Bytes,
    ExitCode,
    SyscallInvocationParams,
    B256,
    CALL_STACK_LIMIT,
};
use rwasm::{Caller, HostError, TrapCode};
use std::{
    cmp::min,
    fmt::{Debug, Display, Formatter},
};

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
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let remaining_fuel = caller.store().remaining_fuel().unwrap_or(u64::MAX);
        let disable_fuel = caller.context().disable_fuel;
        let [hash32_ptr, input_ptr, input_len, fuel16_ptr, state] = caller.stack_pop_n();
        // make sure we have enough fuel for this call
        let fuel16_ptr = fuel16_ptr.as_usize();
        let fuel_limit = if fuel16_ptr > 0 {
            let mut fuel_buffer = [0u8; 16];
            caller.memory_read(fuel16_ptr, &mut fuel_buffer)?;
            let fuel_limit = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
            let _fuel_refund = LittleEndian::read_i64(&fuel_buffer[8..]);
            if fuel_limit > 0 {
                if fuel_limit != u64::MAX && fuel_limit > remaining_fuel && !disable_fuel {
                    return Err(TrapCode::OutOfFuel);
                }
                min(fuel_limit, remaining_fuel)
            } else {
                0
            }
        } else {
            remaining_fuel
        };
        // return resumable error
        caller.context_mut().resumable_context = Some(SysExecResumable {
            params: SyscallInvocationParams {
                code_hash: B256::from(caller.memory_read_fixed::<32>(hash32_ptr.as_usize())?),
                input: Bytes::from(
                    caller.memory_read_vec(input_ptr.as_usize(), input_len.as_usize())?,
                ),
                fuel_limit,
                state: state.as_u32(),
                fuel16_ptr: fuel16_ptr as u32,
            },
            is_root: caller.store().context().call_depth == 0,
        });
        Err(TrapCode::ExecutionHalted)
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        context: &SysExecResumable,
    ) -> (u64, i64, i32) {
        let fuel_limit = context.params.fuel_limit;
        let (fuel_consumed, fuel_refunded, exit_code) = Self::fn_impl(
            caller.context_mut(),
            context.params.code_hash,
            context.params.input.as_ref(),
            fuel_limit,
            context.params.state,
        );
        (fuel_consumed, fuel_refunded, exit_code)
    }

    pub fn fn_impl<I: Into<BytecodeOrHash>>(
        ctx: &mut RuntimeContext,
        code_hash: I,
        input: &[u8],
        fuel_limit: u64,
        state: u32,
    ) -> (u64, i64, i32) {
        // check call depth overflow
        if ctx.call_depth >= CALL_STACK_LIMIT {
            return (fuel_limit, 0, ExitCode::CallDepthOverflow.into_i32());
        }

        let bytecode_or_hash = code_hash.into().with_resolved_hash();

        #[cfg(feature = "wasmtime")]
        {
            use fluentbase_genesis::is_system_precompile_hash;
            let hash = bytecode_or_hash.resolve_hash();
            if is_system_precompile_hash(&hash) {
                let (fuel_consumed, fuel_refunded, exit_code, output) =
                    crate::wasmtime::execute(&hash, input.to_vec(), fuel_limit, state);
                ctx.execution_result.return_data = output;
                return (fuel_consumed, fuel_refunded, exit_code);
            }
        }

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new(bytecode_or_hash)
            .with_input(Bytes::copy_from_slice(input))
            .with_fuel_limit(fuel_limit)
            .with_state(state)
            .with_call_depth(ctx.call_depth + 1)
            .with_disable_fuel(ctx.disable_fuel);

        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call();

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = runtime.remember_runtime(ctx);
        }

        ctx.execution_result.return_data = execution_result.output.clone();

        (
            execution_result.fuel_consumed,
            execution_result.fuel_refunded,
            execution_result.exit_code,
        )
    }
}

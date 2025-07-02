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
use rwasm::{Store, TrapCode, TypedCaller, Value};
use std::{
    cmp::min,
    fmt::{Debug, Display, Formatter},
};

pub struct SyscallExec;

#[derive(Clone)]
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

impl SyscallExec {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let remaining_fuel = caller.remaining_fuel().unwrap_or(u64::MAX);
        let disable_fuel = caller.context(|ctx| ctx.disable_fuel);
        let (hash32_ptr, input_ptr, input_len, fuel16_ptr, state) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as usize,
            params[2].i32().unwrap() as usize,
            params[3].i32().unwrap() as usize,
            params[4].i32().unwrap() as u32,
        );
        // make sure we have enough fuel for this call
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
        let mut code_hash = [0u8; 32];
        caller.memory_read(hash32_ptr, &mut code_hash)?;
        let code_hash = B256::from(code_hash);
        let mut input = vec![0u8; input_len];
        caller.memory_read(input_ptr, &mut input)?;
        let input = Bytes::from(input);
        let is_root = caller.context(|ctx| ctx.call_depth) == 0;
        // return resumable error
        caller.context_mut(|ctx| {
            ctx.resumable_context = Some(SysExecResumable {
                params: SyscallInvocationParams {
                    code_hash,
                    input: input.clone(),
                    fuel_limit,
                    state,
                    fuel16_ptr: fuel16_ptr as u32,
                },
                is_root,
            })
        });
        Err(TrapCode::InterruptionCalled)
    }

    pub fn fn_continue(
        caller: &mut TypedCaller<RuntimeContext>,
        context: &SysExecResumable,
    ) -> (u64, i64, i32) {
        let fuel_limit = context.params.fuel_limit;
        let (fuel_consumed, fuel_refunded, exit_code) = caller.context_mut(|ctx| {
            Self::fn_impl(
                ctx,
                context.params.code_hash,
                context.params.input.as_ref(),
                fuel_limit,
                context.params.state,
            )
        });
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

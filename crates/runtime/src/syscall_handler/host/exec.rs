/// Syscall entry points for deferred execution (exec) within the runtime.
use crate::{
    executor::{default_runtime_executor, RuntimeExecutor},
    RuntimeContext,
};
use alloc::borrow::Cow;
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    BytecodeOrHash, ExitCode, SyscallInvocationParams, B256, CALL_STACK_LIMIT,
};
use rwasm::{StoreTr, TrapCode, Value};
use std::{
    cmp::min,
    fmt::{Debug, Display, Formatter},
};

#[derive(Clone)]
/// Holds parameters required to resume a deferred exec and whether it originated at root depth.
pub struct InterruptionHolder {
    /// Encoded invocation parameters (target code hash, input, fuel, state, pointers).
    pub params: SyscallInvocationParams,
    /// True if the interruption happened at depth 0.
    pub is_root: bool,
}

impl Debug for InterruptionHolder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl Display for InterruptionHolder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

/// Dispatches the exec syscall: validates fuel, captures parameters, and triggers an interruption.
pub fn syscall_exec_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let remaining_fuel = caller.remaining_fuel().unwrap_or(u64::MAX);
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
    let is_root = caller.data().call_depth == 0;
    let params = SyscallInvocationParams {
        code_hash,
        input: input_ptr..(input_ptr + input_len),
        fuel_limit,
        state,
        fuel16_ptr: fuel16_ptr as u32,
    };
    // return resumable error
    caller.data_mut().resumable_context = Some(InterruptionHolder { params, is_root });
    Err(TrapCode::InterruptionCalled)
}

/// Continues an exec after an interruption, executing the delegated call.
pub fn syscall_exec_continue(
    _caller: &mut impl StoreTr<RuntimeContext>,
    _context: &InterruptionHolder,
) -> (u64, i64, i32) {
    unimplemented!("runtime: not supported until we finish zkVM");
    // let fuel_limit = context.params.fuel_limit;
    // let (fuel_consumed, fuel_refunded, exit_code) = caller.context_mut(|ctx| {
    //     syscall_exec_impl(
    //         ctx,
    //         context.params.code_hash,
    //         BytesOrRef::Ref(context.params.input.as_ref()),
    //         fuel_limit,
    //         context.params.state,
    //     )
    // });
    // (fuel_consumed, fuel_refunded, exit_code)
}

/// Executes a nested runtime with the given parameters and merges the result into ctx.
pub fn syscall_exec_impl<I: Into<BytecodeOrHash>>(
    ctx: &mut RuntimeContext,
    code_hash: I,
    input: Cow<'_, [u8]>,
    fuel_limit: u64,
    state: u32,
) -> (u64, i64, i32) {
    // check call depth overflow
    if ctx.call_depth >= CALL_STACK_LIMIT {
        return (fuel_limit, 0, ExitCode::CallDepthOverflow.into_i32());
    }
    // create a new runtime instance with the context
    let ctx2 = RuntimeContext::default()
        .with_fuel_limit(fuel_limit)
        .with_input(input.into_owned())
        .with_state(state)
        .with_call_depth(ctx.call_depth + 1);

    let result = default_runtime_executor().execute(code_hash.into(), ctx2);
    ctx.execution_result.return_data = result.output;
    (result.fuel_consumed, result.fuel_refunded, result.exit_code)
}

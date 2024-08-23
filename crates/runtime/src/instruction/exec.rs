use crate::{Runtime, RuntimeContext};
use fluentbase_types::{Bytes, ExitCode, SyscallInvocationParams, B256};
use rwasm::{
    core::{HostError, Trap},
    Caller,
};
use std::{
    fmt::{Debug, Display, Formatter},
    mem::take,
};

pub struct SyscallExec;

#[derive(Default, Clone)]
pub struct SysExecResumable {
    /// List of delayed invocation params, like exec params (address, code hash, etc.)
    pub params: SyscallInvocationParams,
    /// A depth level of the current call, for root it's always zero
    pub is_root: bool,
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
        caller: Caller<'_, RuntimeContext>,
        hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel: u64,
        state: u32,
    ) -> Result<i32, Trap> {
        Err(SysExecResumable {
            params: SyscallInvocationParams {
                code_hash: B256::from_slice(caller.read_memory(hash32_ptr, 32)?),
                input: Bytes::copy_from_slice(caller.read_memory(input_ptr, input_len)?),
                fuel_limit: fuel,
                state,
            },
            is_root: caller.data().call_depth == 0,
        }
        .into())
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        context: &SysExecResumable,
    ) -> Result<i32, Trap> {
        Ok(Self::fn_impl(
            caller.data_mut(),
            &context.params.code_hash.0,
            context.params.input.as_ref(),
            context.params.fuel_limit,
            context.params.state,
        ))
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        bytecode_hash32: &[u8; 32],
        input: &[u8],
        fuel_limit: u64,
        state: u32,
    ) -> i32 {
        // check call depth overflow
        if ctx.call_depth >= CALL_STACK_LIMIT {
            return ExitCode::CallDepthOverflow.into_i32();
        } else if ctx.fuel.remaining() < fuel_limit {
            return ExitCode::OutOfGas.into_i32();
        }

        // take jzkt from the existing context (we will return it soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");
        let context = take(&mut ctx.context);

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(bytecode_hash32.into())
            .with_input(input.to_vec())
            .with_context(context)
            .with_is_shared(true)
            .with_fuel(fuel_limit)
            .with_jzkt(jzkt)
            .with_state(state)
            .with_depth(ctx.call_depth + 1);
        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call();

        // return jzkt context back
        ctx.jzkt = take(&mut runtime.store.data_mut().jzkt);
        ctx.context = take(&mut runtime.store.data_mut().context);

        // if execution was interrupted,
        if execution_result.interrupted {
            // then we remember this runtime and assign call id into exit code (positive exit code
            // stands for interrupted runtime call id, negative or zero for error)
            execution_result.exit_code = runtime.remember_runtime() as i32;
        } else {
            // charge consumed fuel from parent's context
            ctx.fuel.charge(execution_result.fuel_consumed);
            // increase total fuel consumed and remember return data
            ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        ctx.execution_result.return_data = execution_result.output.clone();

        execution_result.exit_code
    }
}

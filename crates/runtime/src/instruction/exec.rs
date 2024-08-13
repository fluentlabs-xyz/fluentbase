use crate::{Runtime, RuntimeContext};
use fluentbase_codec::{Codec, Encoder};
use fluentbase_types::{Address, Bytes, DelayedInvocationParams, ExitCode, B256};
use rwasm::{
    core::{HostError, Trap},
    Caller,
};
use std::{
    fmt::{Debug, Display, Formatter},
    mem::take,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct SyscallExec;

#[derive(Default, Clone)]
pub struct SysExecResumable {
    /// List of delayed invocation params, like exec params (address, code hash, etc.)
    pub params: DelayedInvocationParams,
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
        address20_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        fuel: u64,
        state: u32,
    ) -> Result<i32, Trap> {
        Err(SysExecResumable {
            params: DelayedInvocationParams {
                code_hash: B256::from_slice(caller.read_memory(hash32_ptr, 32)?),
                address: Address::from_slice(caller.read_memory(address20_ptr, 20)?),
                input: Bytes::copy_from_slice(caller.read_memory(input_ptr, input_len)?),
                fuel_limit: fuel,
                state,
            },
            is_root: caller.data().depth == 0,
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
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // check call depth overflow
        if ctx.depth >= CALL_STACK_LIMIT {
            return ExitCode::CallDepthOverflow.into_i32();
        }

        // take jzkt from the existing context (we will return it soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");
        let context = take(&mut ctx.context);

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(bytecode_hash32.into())
            .with_input(input.to_vec())
            .with_context(context)
            .with_is_shared(true)
            .with_fuel_limit(fuel_limit)
            .with_jzkt(jzkt)
            .with_state(state)
            .with_depth(ctx.depth + 1);
        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call();

        // return jzkt context back
        ctx.jzkt = take(&mut runtime.store.data_mut().jzkt);
        ctx.context = take(&mut runtime.store.data_mut().context);

        // make sure there is no return overflow
        // if return_len > 0 && execution_result.output.len() > return_len as usize {
        //     return (0, ExitCode::OutputOverflow.into_i32());
        // }

        // if execution was interrupted,
        // then we must remember this runtime and assign call id into exit code
        // (positive exit code stands for interrupted runtime call id)
        if execution_result.interrupted {
            execution_result.exit_code = runtime.remember_runtime() as i32;
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        // increase total fuel consumed and remember return data
        ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        ctx.execution_result.return_data = execution_result.output.clone();

        // println!(
        //     "sys_exec_hash ({}), exit_code={}, fuel_consumed={}, elapsed time: {}ms, output={}",
        //     hex::encode(&bytecode_hash32),
        //     execution_result.exit_code,
        //     execution_result.fuel_consumed,
        //     SystemTime::now()
        //         .duration_since(UNIX_EPOCH)
        //         .unwrap()
        //         .as_millis()
        //         - time,
        //     hex::encode(&execution_result.output),
        // );

        execution_result.exit_code
    }
}

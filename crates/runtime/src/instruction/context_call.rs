use crate::{ExecutionResult, Runtime, RuntimeContext};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{
    core::{HostError, Trap},
    Caller,
};
use std::{
    fmt::{Display, Formatter},
    mem::take,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct SyscallContextCall;

#[derive(Debug)]
pub struct SysContextCallResumable {
    pub code_hash32_ptr: u32,
    pub input_ptr: u32,
    pub input_len: u32,
    pub context_ptr: u32,
    pub context_len: u32,
    pub return_ptr: u32,
    pub return_len: u32,
    pub fuel_ptr: u32,
    pub state: u32,
}

pub const CALL_STACK_LIMIT: u32 = 1024;

impl Display for SysContextCallResumable {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl HostError for SysContextCallResumable {}

impl SyscallContextCall {
    pub fn fn_handler<DB: IJournaledTrie>(
        _caller: Caller<'_, RuntimeContext<DB>>,
        code_hash32_ptr: u32,
        input_ptr: u32,
        input_len: u32,
        context_ptr: u32,
        context_len: u32,
        return_ptr: u32,
        return_len: u32,
        fuel_ptr: u32,
        state: u32,
    ) -> Result<i32, Trap> {
        return Err(SysContextCallResumable {
            code_hash32_ptr,
            input_ptr,
            input_len,
            context_ptr,
            context_len,
            return_ptr,
            return_len,
            fuel_ptr,
            state,
        }
        .into());
    }

    pub fn fn_continue<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        state: &SysContextCallResumable,
    ) -> Result<ExitCode, Trap> {
        let bytecode_hash32: [u8; 32] = caller
            .read_memory(state.code_hash32_ptr, 32)?
            .try_into()
            .unwrap();
        let input = caller
            .read_memory(state.input_ptr, state.input_len)?
            .to_vec();
        let context = caller
            .read_memory(state.context_ptr, state.context_len)?
            .to_vec();
        let fuel_data = caller.read_memory(state.fuel_ptr, 4)?;
        let fuel_limit = LittleEndian::read_u32(fuel_data);
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &bytecode_hash32,
            input,
            context,
            state.return_len,
            fuel_limit as u64,
            state.state,
        ) {
            Ok(remaining_fuel) => {
                if state.return_len > 0 {
                    let return_data = caller.data().execution_result.return_data.clone();
                    caller.write_memory(state.return_ptr, &return_data)?;
                }
                let mut fuel_buffer = [0u8; 4];
                LittleEndian::write_u32(&mut fuel_buffer, remaining_fuel as u32);
                caller.write_memory(state.fuel_ptr, &fuel_buffer)?;
                ExitCode::Ok
            }
            Err(err) => err,
        };
        Ok(exit_code)
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        bytecode_hash32: &[u8; 32],
        input: Vec<u8>,
        context: Vec<u8>,
        return_len: u32,
        fuel_limit: u64,
        state: u32,
    ) -> Result<u64, ExitCode> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // check call depth overflow
        if ctx.depth >= CALL_STACK_LIMIT {
            return Err(ExitCode::CallDepthOverflow);
        }

        // take jzkt from the existing context (we will return it back soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");

        // create new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(bytecode_hash32.into())
            .with_input(input)
            .with_context(context)
            .with_state(state)
            .with_fuel_limit(fuel_limit)
            .with_jzkt(jzkt)
            .with_state(state)
            .with_depth(ctx.depth + 1);
        let mut runtime = Runtime::new(ctx2);
        let execution_result = runtime
            .call()
            .unwrap_or_else(|err| ExecutionResult::new_error(Runtime::catch_trap(&err)));

        // return jzkt context back
        ctx.jzkt = take(&mut runtime.store.data_mut().jzkt);

        // make sure there is no return overflow
        if return_len > 0 && execution_result.output.len() > return_len as usize {
            return Err(ExitCode::OutputOverflow);
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        // increase total fuel consumed and remember return data
        ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        ctx.execution_result.return_data = execution_result.output.clone();

        println!(
            "sys_exec_hash ({}), exit_code={}, fuel_consumed={}, elapsed time: {}ms, output={}",
            hex::encode(&bytecode_hash32),
            execution_result.exit_code,
            execution_result.fuel_consumed,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                - time,
            hex::encode(&execution_result.output),
        );

        if execution_result.exit_code != ExitCode::Ok.into_i32() {
            return Err(execution_result.exit_code.into());
        }

        Ok(fuel_limit - execution_result.fuel_consumed)
    }
}

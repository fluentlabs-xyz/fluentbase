use crate::{Runtime, RuntimeContext};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{Address, Bytes, DelegatedExecution, ExitCode, F254, STATE_MAIN};
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

#[derive(Clone)]
pub struct SysExecResumable {
    pub hash32_ptr: u32,
    pub address20_ptr: u32,
    pub input_ptr: u32,
    pub input_len: u32,
    pub context_ptr: u32,
    pub context_len: u32,
    pub return_ptr: u32,
    pub return_len: u32,
    pub fuel_ptr: u32,

    pub delegated_execution: DelegatedExecution,

    /// A depth level of the current call, for root it's always zero
    pub depth_level: u32,
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
        context_ptr: u32,
        context_len: u32,
        return_ptr: u32,
        return_len: u32,
        fuel_ptr: u32,
    ) -> Result<i32, Trap> {
        // it's impossible to interrupt execution on the root level if there is a context
        if caller.data().depth > 0 && context_len > 0 {
            return Err(ExitCode::ContextWriteProtection.into_trap());
        }
        let delegated_execution = DelegatedExecution {
            address: Address::from_slice(caller.read_memory(hash32_ptr, 20)?),
            hash: F254::from_slice(caller.read_memory(hash32_ptr, 32)?),
            input: Bytes::copy_from_slice(caller.read_memory(input_ptr, input_len)?),
            fuel: LittleEndian::read_u32(caller.read_memory(fuel_ptr, 4)?),
        };
        return Err(SysExecResumable {
            hash32_ptr,
            address20_ptr,
            input_ptr,
            input_len,
            context_ptr,
            context_len,
            return_ptr,
            return_len,
            fuel_ptr,
            delegated_execution,
            depth_level: caller.data().depth,
        }
        .into());
    }

    pub fn fn_continue(
        mut caller: Caller<'_, RuntimeContext>,
        state: &SysExecResumable,
    ) -> Result<ExitCode, Trap> {
        let bytecode_hash32: [u8; 32] = caller
            .read_memory(state.hash32_ptr, 32)?
            .try_into()
            .unwrap();
        let input = caller
            .read_memory(state.input_ptr, state.input_len)?
            .to_vec();
        let fuel_data = caller.read_memory(state.fuel_ptr, 4)?;
        let fuel_limit = LittleEndian::read_u32(fuel_data);
        let (remaining_fuel, exit_code) = Self::fn_impl(
            caller.data_mut(),
            &bytecode_hash32,
            &input,
            state.return_len,
            fuel_limit as u64,
        );
        if state.return_len > 0 {
            let return_data = caller.data().execution_result.return_data.clone();
            caller.write_memory(state.return_ptr, &return_data)?;
        }
        let mut fuel_buffer = [0u8; 4];
        LittleEndian::write_u32(&mut fuel_buffer, remaining_fuel as u32);
        caller.write_memory(state.fuel_ptr, &fuel_buffer)?;
        Ok(exit_code)
    }

    pub fn fn_impl(
        ctx: &mut RuntimeContext,
        bytecode_hash32: &[u8; 32],
        input: &[u8],
        return_len: u32,
        fuel_limit: u64,
    ) -> (u64, ExitCode) {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // check call depth overflow
        if ctx.depth >= CALL_STACK_LIMIT {
            return (0, ExitCode::CallDepthOverflow);
        }

        // take jzkt from the existing context (we will return it soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");
        let context = take(&mut ctx.context);

        // create a new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(bytecode_hash32.into())
            .with_input(input.to_vec())
            .with_context(context)
            .with_is_shared(false)
            .with_fuel_limit(fuel_limit)
            .with_jzkt(jzkt)
            .with_state(STATE_MAIN)
            .with_depth(ctx.depth + 1);
        let mut runtime = Runtime::new(ctx2);
        let mut execution_result = runtime.call();

        // return jzkt context back
        ctx.jzkt = take(&mut runtime.store.data_mut().jzkt);
        ctx.context = take(&mut runtime.store.data_mut().context);

        // make sure there is no return overflow
        if return_len > 0 && execution_result.output.len() > return_len as usize {
            return (0, ExitCode::OutputOverflow);
        }

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

        (
            fuel_limit - execution_result.fuel_consumed,
            ExitCode::from(execution_result.exit_code),
        )
    }
}

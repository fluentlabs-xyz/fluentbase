use crate::{ExecutionResult, Runtime, RuntimeContext};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::core::HostError;
use rwasm::{core::Trap, Caller};
use std::fmt::{Display, Formatter};
use std::mem::take;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SysExecHash;

#[derive(Debug)]
pub struct SysExecHashResumable {
    pub bytecode_hash32_offset: u32,
    pub input_offset: u32,
    pub input_len: u32,
    pub return_offset: u32,
    pub return_len: u32,
    pub fuel_offset: u32,
    pub state: u32,
}

impl Display for SysExecHashResumable {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl HostError for SysExecHashResumable {}

impl SysExecHash {
    pub fn fn_handler<DB: IJournaledTrie>(
        _caller: Caller<'_, RuntimeContext<DB>>,
        bytecode_hash32_offset: u32,
        input_offset: u32,
        input_len: u32,
        return_offset: u32,
        return_len: u32,
        fuel_offset: u32,
        state: u32,
    ) -> Result<i32, Trap> {
        return Err(SysExecHashResumable {
            bytecode_hash32_offset,
            input_offset,
            input_len,
            return_offset,
            return_len,
            fuel_offset,
            state,
        }
        .into());
    }

    pub fn fn_continue<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        context: &SysExecHashResumable,
    ) -> Result<i32, Trap> {
        let bytecode_hash32: [u8; 32] = caller
            .read_memory(context.bytecode_hash32_offset, 32)?
            .try_into()
            .unwrap();
        let input = caller
            .read_memory(context.input_offset, context.input_len)?
            .to_vec();
        let fuel_data = caller.read_memory(context.fuel_offset, 4)?;
        let fuel_limit = LittleEndian::read_u32(fuel_data);
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &bytecode_hash32,
            input,
            context.return_len,
            fuel_limit as u64,
            context.state,
        ) {
            Ok(remaining_fuel) => {
                if context.return_len > 0 {
                    let return_data = caller.data().execution_result.return_data.clone();
                    caller.write_memory(context.return_offset, &return_data)?;
                }
                let mut fuel_buffer = [0u8; 4];
                LittleEndian::write_u32(&mut fuel_buffer, remaining_fuel as u32);
                caller.write_memory(context.fuel_offset, &fuel_buffer)?;
                ExitCode::Ok.into_i32()
            }
            Err(err) => err,
        };
        Ok(exit_code)
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        bytecode_hash32: &[u8; 32],
        input: Vec<u8>,
        return_len: u32,
        fuel_limit: u64,
        state: u32,
    ) -> Result<u64, i32> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // take jzkt from the existing context (we will return it back soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");

        // check call depth overflow
        if ctx.depth + 1 >= 1024 {
            return Err(ExitCode::CallDepthOverflow.into_i32());
        }

        // create new runtime instance with the context
        let ctx2 = RuntimeContext::new_with_hash(bytecode_hash32.into())
            .with_input(input)
            .with_state(state)
            .with_is_shared(false)
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
            return Err(ExitCode::OutputOverflow.into_i32());
        }

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        // increase total fuel consumed and remember return data
        ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        ctx.execution_result.return_data = execution_result.output.clone();

        println!(
            "sys_exec_hash ({}), elapsed time: {}ms",
            hex::encode(&bytecode_hash32),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                - time
        );

        if execution_result.exit_code != ExitCode::Ok.into_i32() {
            return Err(execution_result.exit_code);
        }

        Ok(fuel_limit - execution_result.fuel_consumed)
    }
}

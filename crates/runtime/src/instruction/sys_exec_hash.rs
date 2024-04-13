use crate::{Runtime, RuntimeContext};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};
use std::mem::take;

pub struct SysExecHash;

impl SysExecHash {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        bytecode_hash32_offset: u32,
        input_offset: u32,
        input_len: u32,
        return_offset: u32,
        return_len: u32,
        fuel_offset: u32,
        state: u32,
    ) -> Result<i32, Trap> {
        let bytecode_hash32: [u8; 32] = caller
            .read_memory(bytecode_hash32_offset, 32)?
            .try_into()
            .unwrap();
        let input = caller.read_memory(input_offset, input_len)?.to_vec();
        let fuel_data = caller.read_memory(fuel_offset, 4)?;
        let fuel = LittleEndian::read_u32(fuel_data);
        let exit_code = match Self::fn_impl(
            caller.data_mut(),
            &bytecode_hash32,
            input,
            return_len,
            fuel as u64,
            state,
        ) {
            Ok(remaining_fuel) => {
                if return_len > 0 {
                    let return_data = caller.data().execution_result.return_data.clone();
                    caller.write_memory(return_offset, &return_data)?;
                }
                let mut fuel_buffer = [0u8; 4];
                LittleEndian::write_u32(&mut fuel_buffer, remaining_fuel as u32);
                caller.write_memory(fuel_offset, &fuel_buffer)?;
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
        let import_linker = Runtime::new_sovereign_linker();

        // load bytecode based on the preimage provided
        let bytecode = ctx.jzkt().preimage(bytecode_hash32);

        // take jzkt from the existing context (we will return it back soon)
        let jzkt = take(&mut ctx.jzkt).expect("jzkt is not initialized");

        // create new runtime instance with the context
        let ctx2 = RuntimeContext::new(bytecode)
            .with_input(input)
            .with_state(state)
            .with_is_shared(false)
            .with_fuel_limit(fuel_limit)
            .with_catch_trap(true)
            .with_jzkt(jzkt)
            .with_state(state);
        let mut runtime = match Runtime::new(ctx2, import_linker) {
            Err(err) => {
                return Err(Runtime::catch_trap(&err));
            }
            Ok(runtime) => runtime,
        };
        let execution_result = match runtime.call() {
            Err(err) => {
                return Err(Runtime::catch_trap(&err));
            }
            Ok(execution_result) => execution_result,
        };

        // make sure there is no return overflow
        if return_len > 0 && execution_result.output.len() > return_len as usize {
            return Err(ExitCode::OutputOverflow.into_i32());
        }

        // return jzkt context back
        ctx.jzkt = take(&mut runtime.store.data_mut().jzkt);

        // TODO(dmitry123): "do we need to put any fuel penalties for failed calls?"

        // increase total fuel consumed and remember return data
        ctx.execution_result.fuel_consumed += execution_result.fuel_consumed;
        ctx.execution_result.return_data = execution_result.output.clone();

        Ok(fuel_limit - execution_result.fuel_consumed)
    }
}

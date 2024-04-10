use crate::{Runtime, RuntimeContext};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{ExitCode, IJournaledTrie, STATE_MAIN};
use rwasm::{core::Trap, Caller};

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
            fuel,
            state,
        ) {
            Ok((return_data, remaining_fuel)) => {
                if return_len > 0 {
                    caller.write_memory(return_offset, &return_data)?;
                }
                let mut fuel_buffer = [0u8; 4];
                LittleEndian::write_u32(&mut fuel_buffer, remaining_fuel);
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
        fuel_limit: u32,
        _state: u32,
    ) -> Result<(Vec<u8>, u32), i32> {
        let import_linker = Runtime::<DB>::new_sovereign_linker();
        let jzkt = ctx.jzkt.as_mut().unwrap();
        let bytecode = jzkt.preimage(bytecode_hash32);
        let next_ctx = RuntimeContext::new(bytecode)
            .with_input(input)
            .with_state(STATE_MAIN)
            .with_is_shared(false)
            .with_fuel_limit(fuel_limit)
            .with_jzkt(ctx.jzkt.clone().unwrap());
        let execution_result = Runtime::<DB>::run_with_context(next_ctx, import_linker)
            .map_err(|_| ExitCode::TransactError.into_i32())?;
        let fuel_consumed = execution_result.fuel_consumed().unwrap_or_default() as u32;
        let output = execution_result.data().output();
        if return_len > 0 && output.len() > return_len as usize {
            return Err(ExitCode::OutputOverflow.into_i32());
        }
        if execution_result.data().exit_code != ExitCode::Ok.into_i32() {
            return Err(execution_result.data().exit_code);
        }
        ctx.consumed_fuel += fuel_consumed;
        ctx.return_data = output.clone();
        Ok((output.clone(), fuel_limit - fuel_consumed))
    }
}

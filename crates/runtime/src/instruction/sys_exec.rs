use crate::{Runtime, RuntimeContext};
use byteorder::{BigEndian, ByteOrder};
use fluentbase_types::{ExitCode, STATE_MAIN};
use rwasm::{common::Trap, Caller};

pub struct SysExec;

impl SysExec {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        code_offset: u32,
        code_len: u32,
        input_offset: u32,
        input_len: u32,
        return_offset: u32,
        return_len: u32,
        fuel_offset: u32,
        state: u32,
    ) -> Result<i32, Trap> {
        let code = caller.read_memory(code_offset, code_len).to_vec();
        let input = caller.read_memory(input_offset, input_len).to_vec();
        let fuel = BigEndian::read_u32(caller.read_memory(fuel_offset, 4));
        let exit_code = match Self::fn_impl(caller.data_mut(), code, input, return_len, fuel, state)
        {
            Ok((return_data, remaining_fuel)) => {
                if return_len > 0 {
                    caller.write_memory(return_offset, &return_data);
                }
                let mut fuel_buffer = [0u8; 4];
                BigEndian::write_u32(&mut fuel_buffer, remaining_fuel);
                caller.write_memory(fuel_offset, &fuel_buffer);
                ExitCode::Ok
            }
            Err(err) => err,
        };
        Ok(exit_code.into_i32())
    }

    pub fn fn_impl<T>(
        ctx: &mut RuntimeContext<T>,
        bytecode: Vec<u8>,
        input: Vec<u8>,
        return_len: u32,
        fuel_limit: u32,
        _state: u32,
    ) -> Result<(Vec<u8>, u32), ExitCode> {
        let import_linker = Runtime::<()>::new_shared_linker();
        let mut next_ctx = RuntimeContext::new(bytecode);
        next_ctx
            .with_input(input)
            .with_state(STATE_MAIN)
            .with_is_shared(true)
            .with_fuel_limit(fuel_limit);
        let execution_result = Runtime::<()>::run_with_context(next_ctx, &import_linker)
            .map_err(|_| ExitCode::TransactError)?;
        let fuel_consumed = execution_result.fuel_consumed().unwrap_or_default() as u32;
        let output = execution_result.data().output();
        if return_len > 0 && output.len() > return_len as usize {
            return Err(ExitCode::OutputOverflow);
        }
        if execution_result.data().exit_code != ExitCode::Ok.into_i32() {
            return Err(ExitCode::from(execution_result.data().exit_code));
        }
        ctx.consumed_fuel += fuel_consumed;
        ctx.return_data = output.clone();
        Ok((output.clone(), fuel_limit - fuel_consumed))
    }
}

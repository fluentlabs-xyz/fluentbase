use crate::{Runtime, RuntimeContext};
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
        fuel: u32,
    ) -> Result<i32, Trap> {
        let code = caller.read_memory(code_offset, code_len).to_vec();
        let input = caller.read_memory(input_offset, input_len).to_vec();
        let exit_code = match Self::fn_impl(caller.data_mut(), code, input, return_len, fuel) {
            Ok(return_data) => {
                if return_offset > 0 && return_len > 0 {
                    caller.write_memory(return_offset, &return_data);
                }
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
        fuel: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        let import_linker = Runtime::<()>::new_shared_linker();
        let next_ctx = RuntimeContext::new(bytecode)
            .with_input(input)
            .with_state(STATE_MAIN)
            .with_is_shared(true)
            .with_fuel_limit(fuel);
        let execution_result = Runtime::<()>::run_with_context(next_ctx, &import_linker)
            .map_err(|_| ExitCode::TransactError)?;
        let output = execution_result.data().output();
        if output.len() > return_len as usize {
            return Err(ExitCode::OutputOverflow);
        }
        ctx.return_data = output.clone();
        Ok(output.clone())
    }
}

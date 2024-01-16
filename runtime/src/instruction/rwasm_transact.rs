use crate::{ExitCode, Runtime, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysExec;

impl SysExec {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        code_offset: u32,
        code_len: u32,
        input_offset: u32,
        input_len: u32,
        output_offset: u32,
        output_len: u32,
        state: u32,
        fuel: u32,
    ) -> Result<i32, Trap> {
        let code = caller.read_memory(code_offset, code_len);
        let input = caller.read_memory(input_offset, input_len);
        match Self::fn_impl(code, input, state, fuel, output_len) {
            Ok(output) => {
                caller.write_memory(output_offset, &output);
                Ok(0)
            }
            Err(err) => Ok(err.into()),
        }
    }

    pub fn fn_impl(
        code: &[u8],
        input: &[u8],
        state: u32,
        fuel: u32,
        output_len: u32,
    ) -> Result<Vec<u8>, i32> {
        // TODO: "we probably need custom linker here with reduced host calls number"
        // TODO: "make sure there is no panic inside runtime"
        let import_linker = Runtime::<()>::new_linker();
        let ctx = RuntimeContext::new(code)
            .with_input(input.to_vec())
            .with_state(state)
            .with_fuel_limit(fuel);
        let execution_result = Runtime::<()>::run_with_context(ctx, &import_linker)
            .map_err(|_| ExitCode::TransactError as i32)?;
        let output = execution_result.data().output();
        // TODO: "this is not a good way for handling this"
        if output_len < output.len() as u32 {
            return Err(output.len() as i32);
        }
        // TODO: "exit code from shared apps can't be greater than 0"
        if execution_result.data().exit_code() > 0 {
            return Err(ExitCode::TransactError as i32);
        }
        Ok(output.clone())
    }
}

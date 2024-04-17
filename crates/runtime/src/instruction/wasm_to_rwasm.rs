use crate::RuntimeContext;
use fluentbase_types::{wasm2rwasm, ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct WasmToRwasm;

impl WasmToRwasm {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        input_offset: u32,
        input_len: u32,
        output_offset: u32,
        output_len: u32,
    ) -> Result<i32, Trap> {
        let wasm_binary = caller.read_memory(input_offset, input_len)?;
        let rwasm_binary = Self::fn_impl(wasm_binary).map_err(|v| v.into_trap())?;
        if output_len > 0 {
            caller.write_memory(output_offset, &rwasm_binary[0..output_len as usize])?;
        }
        caller.data_mut().execution_result.return_data = rwasm_binary;
        Ok(ExitCode::Ok.into_i32())
    }

    pub fn fn_impl(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
        wasm2rwasm(wasm_binary)
    }
}

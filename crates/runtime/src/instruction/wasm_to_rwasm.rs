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
        let wasm_binary = caller.read_memory(input_offset, input_len)?.to_vec();
        let rwasm_binary = Self::fn_impl(caller.data_mut(), &wasm_binary, output_len)
            .map_err(|v| v.into_trap())?;
        if output_len > 0 {
            caller.write_memory(output_offset, &rwasm_binary[0..output_len as usize])?;
        }
        Ok(ExitCode::Ok.into_i32())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        wasm_binary: &[u8],
        output_len: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        let rwasm_binary = wasm2rwasm(wasm_binary)?;
        if output_len > 0 && output_len < rwasm_binary.len() as u32 {
            return Err(ExitCode::OutputOverflow);
        }
        ctx.execution_result.return_data = rwasm_binary.clone();
        Ok(rwasm_binary)
    }
}

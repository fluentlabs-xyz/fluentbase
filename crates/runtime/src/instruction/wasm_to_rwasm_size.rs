use crate::instruction::wasm_to_rwasm::WasmToRwasm;
use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct WasmToRwasmSize;

impl WasmToRwasmSize {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        input_offset: u32,
        input_len: u32,
    ) -> Result<i32, Trap> {
        let wasm_binary = caller.read_memory(input_offset, input_len)?.to_vec();
        Self::fn_impl(caller.data_mut(), &wasm_binary).map_err(|err| err.into_trap())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        wasm_binary: &[u8],
    ) -> Result<i32, ExitCode> {
        let size = WasmToRwasm::fn_impl(ctx, wasm_binary, 0)?.len();
        Ok(size as i32)
    }
}

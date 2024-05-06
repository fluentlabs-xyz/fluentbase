use crate::RuntimeContext;
use fluentbase_types::SysFuncIdx::SYS_STATE;
use fluentbase_types::{
    create_sovereign_import_linker, ExitCode, IJournaledTrie, STATE_DEPLOY, STATE_MAIN,
};
use rwasm::engine::bytecode::Instruction;
use rwasm::engine::{RwasmConfig, StateRouterConfig};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};
use rwasm::{core::Trap, Caller};

pub struct WasmToRwasm;

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(SYS_STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_sovereign_import_linker()),
        wrap_import_functions: true,
    });
    let rwasm_module = RwasmModule::compile_with_config(wasm_binary, &config)
        .map_err(|_| ExitCode::CompilationError)?;
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}

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

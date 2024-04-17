use crate::SysFuncIdx::SYS_STATE;
use crate::{create_sovereign_import_linker, ExitCode, STATE_DEPLOY, STATE_MAIN};
use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use rwasm::engine::bytecode::Instruction;
use rwasm::engine::{RwasmConfig, StateRouterConfig};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};
use rwasm::Error;

#[inline(always)]
pub fn rwasm_module(wasm_binary: &[u8]) -> Result<RwasmModule, Error> {
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
    RwasmModule::compile_with_config(wasm_binary, &config)
}

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let rwasm_module = rwasm_module(wasm_binary);
    if rwasm_module.is_err() {
        return Err(ExitCode::CompilationError);
    }
    let rwasm_module = rwasm_module.unwrap();
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}

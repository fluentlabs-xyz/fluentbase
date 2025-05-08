use crate::{create_import_linker, sys_func_idx::SysFuncIdx, Bytes, STATE_DEPLOY, STATE_MAIN};
use alloc::{boxed::Box, string::ToString, vec};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    Config,
    Error,
};

pub struct RwasmCompilationResult {
    pub rwasm_bytecode: Bytes,
    pub constructor_params: Bytes,
}

pub fn default_compilation_config() -> Config {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(SysFuncIdx::STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_import_linker()),
        wrap_import_functions: true,
        translate_drop_keep: false,
        allow_malformed_entrypoint_func_type: false,
        use_32bit_mode: false,
        builtins_consume_fuel: true,
    });
    config
}

pub fn compile_wasm_to_rwasm_with_config(
    wasm_binary: &[u8],
    config: Config,
) -> Result<RwasmCompilationResult, Error> {
    let (rwasm_module, constructor_params) =
        RwasmModule::compile_and_retrieve_input(wasm_binary, &config)?;
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(RwasmCompilationResult {
        rwasm_bytecode: rwasm_bytecode.into(),
        constructor_params: constructor_params.into(),
    })
}

pub fn compile_wasm_to_rwasm(wasm_binary: &[u8]) -> Result<RwasmCompilationResult, Error> {
    compile_wasm_to_rwasm_with_config(wasm_binary, default_compilation_config())
}

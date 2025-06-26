use crate::{sys_func_idx::SysFuncIdx, STATE_DEPLOY, STATE_MAIN};
use alloc::{boxed::Box, vec::Vec};
use rwasm::{CompilationConfig, CompilationError, Opcode, RwasmModule, StateRouterConfig};

mod block_fuel;
mod import_linker;

pub use block_fuel::*;
pub use import_linker::*;

pub struct RwasmCompilationResult {
    pub rwasm_module: RwasmModule,
    pub constructor_params: Vec<u8>,
}

pub fn default_compilation_config() -> CompilationConfig {
    let linker = create_import_linker();
    CompilationConfig::default()
        .with_state_router(StateRouterConfig {
            states: Box::new([("deploy".into(), STATE_DEPLOY), ("main".into(), STATE_MAIN)]),
            opcode: Some(Opcode::Call(SysFuncIdx::STATE as u32)),
        })
        .with_import_linker(linker)
        .with_allow_malformed_entrypoint_func_type(false)
        .with_builtins_consume_fuel(true)
}

pub fn compile_wasm_to_rwasm_with_config(
    wasm_binary: &[u8],
    config: CompilationConfig,
) -> Result<RwasmCompilationResult, CompilationError> {
    let (rwasm_module, constructor_params) = RwasmModule::compile(config, wasm_binary)?;
    Ok(RwasmCompilationResult {
        rwasm_module,
        constructor_params: constructor_params.into(),
    })
}

pub fn compile_wasm_to_rwasm(
    wasm_binary: &[u8],
) -> Result<RwasmCompilationResult, CompilationError> {
    compile_wasm_to_rwasm_with_config(wasm_binary, default_compilation_config())
}

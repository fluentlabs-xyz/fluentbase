use crate::{import_linker_v1_preview, SysFuncIdx, STATE_DEPLOY, STATE_MAIN};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use fluentbase_types::{is_engine_metered_precompile, is_execute_using_system_runtime, Address};
use rwasm::{
    CompilationConfig, CompilationError, ImportLinker, Opcode, RwasmModule, StateRouterConfig,
};
use rwasm_core::{N_DEFAULT_MAX_MEMORY_PAGES, N_MAX_ALLOWED_MEMORY_PAGES};

pub struct RwasmCompilationResult {
    pub rwasm_module: RwasmModule,
    pub constructor_params: Vec<u8>,
}

pub fn default_compilation_config() -> CompilationConfig {
    let linker = import_linker_v1_preview();
    default_compilation_config_with_linker(linker)
}

pub fn default_compilation_config_with_linker(
    import_linker: Arc<ImportLinker>,
) -> CompilationConfig {
    CompilationConfig::default()
        .with_state_router(StateRouterConfig {
            states: Box::new([("deploy".into(), STATE_DEPLOY), ("main".into(), STATE_MAIN)]),
            opcode: Some(Opcode::Call(SysFuncIdx::STATE as u32)),
        })
        .with_import_linker(import_linker)
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

pub fn compile_rwasm_maybe_system(
    contract_address: &Address,
    wasm_bytecode: &[u8],
) -> Result<RwasmCompilationResult, CompilationError> {
    let is_system_runtime = is_execute_using_system_runtime(contract_address);

    // Most system precompiles manage fuel internally via `_charge_fuel` syscall.
    // However, some precompiles (NITRO_VERIFIER, OAUTH2_VERIFIER, WASM_RUNTIME,
    // WEBAUTHN_VERIFIER) don't self-meter, so they need fuel instrumentation.
    let should_charge_fuel = is_engine_metered_precompile(contract_address);

    let config = default_compilation_config()
        .with_consume_fuel(should_charge_fuel)
        .with_builtins_consume_fuel(should_charge_fuel)
        .with_max_allowed_memory_pages(if is_system_runtime {
            N_MAX_ALLOWED_MEMORY_PAGES
        } else {
            N_DEFAULT_MAX_MEMORY_PAGES
        })
        .with_allow_malformed_entrypoint_func_type(is_system_runtime);

    compile_wasm_to_rwasm_with_config(wasm_bytecode, config.clone())
}

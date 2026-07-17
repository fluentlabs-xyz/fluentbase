use crate::{import_linker_v1_preview, SysFuncIdx, STATE_DEPLOY, STATE_MAIN};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use fluentbase_types::{
    is_execute_using_system_runtime, Address, CompilationBackend, CompilationConfigFingerprint,
};
use rwasm::{
    CompilationConfig, CompilationError, ImportLinker, Opcode, RwasmModule, StateRouterConfig,
};
use rwasm_core::{N_DEFAULT_MAX_MEMORY_PAGES, N_MAX_ALLOWED_MEMORY_PAGES};

pub struct RwasmCompilationResult {
    pub rwasm_module: RwasmModule,
    pub constructor_params: Vec<u8>,
    pub config_fingerprint: CompilationConfigFingerprint,
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
        .with_consume_fuel_for_bulk_ops(true)
        .with_builtins_consume_fuel(true)
}

pub fn compile_wasm_to_rwasm_with_config(
    wasm_binary: &[u8],
    config: CompilationConfig,
) -> Result<RwasmCompilationResult, CompilationError> {
    let config_fingerprint = CompilationConfigFingerprint::from_config(
        &config,
        CompilationBackend::Rwasm,
        Address::ZERO,
    );
    let (rwasm_module, constructor_params) = RwasmModule::compile(config, wasm_binary)?;
    Ok(RwasmCompilationResult {
        rwasm_module,
        constructor_params: constructor_params.into(),
        config_fingerprint,
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
    let config = compilation_config_for_contract_address(contract_address);
    let config_fingerprint = CompilationConfigFingerprint::from_config(
        &config,
        CompilationBackend::Rwasm,
        *contract_address,
    );
    let (rwasm_module, constructor_params) = RwasmModule::compile(config, wasm_bytecode)?;
    Ok(RwasmCompilationResult {
        rwasm_module,
        constructor_params: constructor_params.into(),
        config_fingerprint,
    })
}

pub fn compilation_config_for_contract_address(contract_address: &Address) -> CompilationConfig {
    let is_system_runtime = is_execute_using_system_runtime(contract_address);

    // Most system precompiles manage fuel internally via `_charge_fuel` syscall.
    // However, some precompiles (NITRO_VERIFIER, OAUTH2_VERIFIER, WASM_RUNTIME,
    // WEBAUTHN_VERIFIER) don't self-meter, so they need fuel instrumentation.
    //
    // P.S: Disabled since we don't want to charge fuel instructions
    let should_charge_fuel = false; // is_engine_metered_precompile(contract_address);

    default_compilation_config()
        .with_consume_fuel(should_charge_fuel)
        .with_consume_fuel_for_bulk_ops(!is_system_runtime)
        .with_consume_fuel_for_params_and_locals(!is_system_runtime)
        .with_builtins_consume_fuel(should_charge_fuel)
        .with_max_allowed_memory_pages(if is_system_runtime {
            N_MAX_ALLOWED_MEMORY_PAGES
        } else {
            N_DEFAULT_MAX_MEMORY_PAGES
        })
        .with_allow_malformed_entrypoint_func_type(is_system_runtime)
}

pub fn compilation_config_fingerprint_for_contract_address(
    contract_address: &Address,
) -> CompilationConfigFingerprint {
    let config = compilation_config_for_contract_address(contract_address);
    CompilationConfigFingerprint::from_config(&config, CompilationBackend::Rwasm, *contract_address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_meters_bulk_ops() {
        let config = default_compilation_config();

        assert!(config.consume_fuel);
        assert!(config.consume_fuel_for_bulk_ops);
    }

    #[test]
    fn fingerprints_split_address_dependent_runtime_policy() {
        let system_config = default_compilation_config()
            .with_consume_fuel(false)
            .with_consume_fuel_for_bulk_ops(false)
            .with_consume_fuel_for_params_and_locals(false)
            .with_builtins_consume_fuel(false)
            .with_max_allowed_memory_pages(rwasm_core::N_MAX_ALLOWED_MEMORY_PAGES)
            .with_allow_malformed_entrypoint_func_type(true);
        let contract_config = default_compilation_config()
            .with_consume_fuel(false)
            .with_consume_fuel_for_bulk_ops(true)
            .with_consume_fuel_for_params_and_locals(true)
            .with_builtins_consume_fuel(false)
            .with_max_allowed_memory_pages(rwasm_core::N_DEFAULT_MAX_MEMORY_PAGES)
            .with_allow_malformed_entrypoint_func_type(false);

        let system = CompilationConfigFingerprint::from_config(
            &system_config,
            CompilationBackend::Rwasm,
            fluentbase_types::PRECOMPILE_EVM_RUNTIME,
        );
        let contract = CompilationConfigFingerprint::from_config(
            &contract_config,
            CompilationBackend::Rwasm,
            Address::repeat_byte(0x11),
        );

        assert_ne!(system, contract);
        assert_ne!(system.identity_hash(), contract.identity_hash());
        assert_eq!(system.stable_bytes(), system.stable_bytes());
    }
}

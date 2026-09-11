#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    default_compilation_config, rwasm_core::RwasmModule, system_entrypoint, ExitCode, SystemAPI,
    RWASM_MAX_CODE_SIZE,
};

/// Compiles user-supplied Wasm into rWasm during deployment.
///
/// This runtime is engine-metered (`ENGINE_METERED_PRECOMPILES`): the execution engine charges
/// every instruction the compiler executes, so the deployment cost already scales with the work
/// done on the input. No static per-byte fee is charged on top of that, otherwise the same
/// compilation would be paid for twice.
pub fn deploy_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let wasm_binary = sdk.bytes_input();

    // Compile wasm into rwasm bytecode
    let config = default_compilation_config();
    let (result, constructor_params) =
        RwasmModule::compile(config, &wasm_binary).map_err(|_| ExitCode::MalformedBuiltinParams)?;

    // The resulting binary size should not exceed RWASM_MAX_CODE_SIZE constant
    let rwasm_binary = result.serialize();
    if rwasm_binary.len() > RWASM_MAX_CODE_SIZE {
        return Err(ExitCode::CreateContractSizeLimit);
    }

    // Write output with constructor params
    sdk.write(&rwasm_binary);
    let constructor_params = constructor_params.into_vec();
    sdk.write(&constructor_params);
    Ok(())
}

pub fn main_entry<SDK: SystemAPI>(_: &mut SDK) -> Result<(), ExitCode> {
    Err(ExitCode::UnreachableCodeReached)
}

system_entrypoint!(main_entry, deploy_entry);

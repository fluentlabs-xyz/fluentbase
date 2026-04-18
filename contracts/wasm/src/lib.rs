#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    default_compilation_config, rwasm_core::RwasmModule, system_entrypoint, ExitCode, SystemAPI,
    FUEL_DENOM_RATE, RWASM_MAX_CODE_SIZE,
};

/// An average overhead per each Wasm byte in fuel cost (~2.5 gas per byte).
const WASM_COMPILATION_OVERHEAD_FUEL_PER_BYTE: u32 = (50 * FUEL_DENOM_RATE) as u32;

pub fn deploy_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // Wasm compilation fuel cost is ~50k fuel per one byte (~2500gas/b)
    let fuel_cost = sdk
        .input_size()
        .saturating_mul(WASM_COMPILATION_OVERHEAD_FUEL_PER_BYTE);
    sdk.charge_fuel(fuel_cost as u64);

    // Read wasm binary once we charged enough gas for input
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

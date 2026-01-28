#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice, default_compilation_config, rwasm_core::RwasmModule, system_entrypoint, ExitCode,
    SharedAPI, RWASM_MAX_CODE_SIZE,
};

pub fn deploy_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_length = sdk.input_size();
    let mut wasm_binary = alloc_slice(input_length as usize);
    sdk.read(&mut wasm_binary, 0);
    let config = default_compilation_config();
    let (result, constructor_params) = RwasmModule::compile(config, &wasm_binary).unwrap();
    let rwasm_binary = result.serialize();
    #[cfg(feature = "testing-enabled")]
    const RWASM_MAX_CODE_SIZE: usize = 2_500_000;
    if rwasm_binary.len() > RWASM_MAX_CODE_SIZE {
        panic!("max code size exceeded {}>{}", rwasm_binary.len(), RWASM_MAX_CODE_SIZE);
    }
    sdk.write(&rwasm_binary);
    let constructor_params = constructor_params.into_vec();
    sdk.write(&constructor_params);
    Ok(())
}

pub fn main_entry<SDK: SharedAPI>(_: &mut SDK) -> Result<(), ExitCode> {
    Err(ExitCode::UnreachableCodeReached)
}

system_entrypoint!(main_entry, deploy_entry);

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, default_compilation_config, entrypoint, rwasm_core, SharedAPI};

pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let input_length = sdk.input_size();
    let mut wasm_binary = alloc_slice(input_length as usize);
    sdk.read(&mut wasm_binary, 0);
    let config = default_compilation_config();
    let (result, constructor_params) =
        rwasm_core::RwasmModule::compile(config, &wasm_binary).unwrap();
    let rwasm_binary = result.serialize();
    sdk.write(&rwasm_binary);
    let constructor_params = constructor_params.into_vec();
    sdk.write(&constructor_params);
}

pub fn main_entry(_: impl SharedAPI) {}

entrypoint!(main_entry, deploy_entry);

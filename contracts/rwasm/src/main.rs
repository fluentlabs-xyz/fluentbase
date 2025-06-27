#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, ContextReader, ExitCode, SharedAPI, SYSTEM_ADDRESS};
use rwasm::{CompilationConfig, RwasmModule};


pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let caller = sdk.context().contract_caller();

    if caller == SYSTEM_ADDRESS {
        sdk.exit(ExitCode::Err);
    }

    let input_length = sdk.input_size();
    let mut wasm_binary = alloc_slice(input_length as usize);
    sdk.read(&mut wasm_binary, 0);
    let (result, params) =
        RwasmModule::compile(CompilationConfig::default(), &wasm_binary).unwrap();

    let mut rwasm_binary = result.serialize();

    let mut output = rwasm_binary.len().to_le_bytes().to_vec();
    output.append(&mut rwasm_binary);
    output.append(&mut params.into_vec());

    sdk.write(output.as_ref());
}

pub fn main_entry(_: impl SharedAPI) {}

entrypoint!(main_entry, deploy_entry);

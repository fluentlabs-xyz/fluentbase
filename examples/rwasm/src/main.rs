#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};
use rwasm::{CompilationConfig, RwasmModule};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let wasm_binary = sdk.input();
    let (rwasm_module, _) =
        RwasmModule::compile(CompilationConfig::default(), wasm_binary.as_ref())
            .expect("failed to compile");
    let rwasm_bytecode = rwasm_module.serialize();
    sdk.write(&rwasm_bytecode);
}

entrypoint!(main_entry);

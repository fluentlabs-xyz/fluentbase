#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, create_import_linker, entrypoint, SharedAPI};
use rwasm::legacy::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let wasm_binary = sdk.input();
    let import_linker = create_import_linker();
    let rwasm_module =
        RwasmModule::compile(wasm_binary.as_ref(), Some(import_linker)).expect("failed to compile");
    let encoded_length = rwasm_module.encoded_length();
    let rwasm_bytecode = alloc_slice(encoded_length);
    let mut binary_format_writer = BinaryFormatWriter::new(rwasm_bytecode);
    let n_bytes = rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rWASM");
    assert_eq!(n_bytes, encoded_length, "encoded bytes mismatch");
    sdk.write(rwasm_bytecode);
}

entrypoint!(main_entry);

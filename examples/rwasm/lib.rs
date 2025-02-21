#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, create_import_linker, func_entrypoint, SharedAPI};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

pub fn main(mut sdk: impl SharedAPI) {
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

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;
    use std::str::from_utf8_unchecked;

    #[test]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("../greeting/lib.wasm");
        let sdk = TestingContext::default().with_input(greeting_bytecode);
        main(sdk.clone());
        let output = sdk.take_output();
        let module = RwasmModule::new(&output).unwrap();
        assert!(module.code_section.len() > 0);
        assert!(unsafe { from_utf8_unchecked(&module.memory_section).contains("Hello, World") })
    }
}

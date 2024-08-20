#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    create_import_linker,
    derive::Contract,
    SharedAPI,
};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

#[derive(Contract)]
struct RWASM<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> RWASM<SDK> {
    fn deploy(&mut self) {
        // any custom deployment logic here
    }
    fn main(&mut self) {
        let input_size = self.sdk.input_size() as usize;
        let wasm_binary = alloc_slice(input_size);
        self.sdk.read(wasm_binary, 0);
        let import_linker = create_import_linker();
        let rwasm_module =
            RwasmModule::compile(wasm_binary, Some(import_linker)).expect("failed to compile");
        let encoded_length = rwasm_module.encoded_length();
        let rwasm_bytecode = alloc_slice(encoded_length);
        let mut binary_format_writer = BinaryFormatWriter::new(rwasm_bytecode);
        let n_bytes = rwasm_module
            .write_binary(&mut binary_format_writer)
            .expect("failed to encode rWASM");
        assert_eq!(n_bytes, encoded_length, "encoded bytes mismatch");
        self.sdk.write(rwasm_bytecode);
    }
}

basic_entrypoint!(RWASM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};
    use std::str::from_utf8_unchecked;

    #[test]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("../greeting/lib.wasm");
        let native_sdk = TestingContext::empty().with_input(greeting_bytecode);
        let sdk = JournalState::empty(native_sdk.clone());
        let mut rwasm = RWASM::new(sdk);
        rwasm.deploy();
        rwasm.main();
        let output = native_sdk.take_output();
        let module = RwasmModule::new(&output).unwrap();
        assert!(module.code_section.len() > 0);
        assert!(unsafe { from_utf8_unchecked(&module.memory_section).contains("Hello, World") })
    }
}

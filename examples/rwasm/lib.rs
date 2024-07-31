#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    create_sovereign_import_linker,
    derive::Contract,
    NativeAPI,
    SharedAPI,
};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

#[derive(Contract)]
struct RWASM<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> RWASM<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        let input_size = self.sdk.native_sdk().input_size() as usize;
        let wasm_binary = alloc_slice(input_size);
        self.sdk.native_sdk().read(wasm_binary, 0);
        let import_linker = create_sovereign_import_linker();
        let rwasm_module =
            RwasmModule::compile(wasm_binary, Some(import_linker)).expect("failed to compile");
        let encoded_length = rwasm_module.encoded_length();
        let rwasm_bytecode = alloc_slice(encoded_length);
        let mut binary_format_writer = BinaryFormatWriter::new(rwasm_bytecode);
        let n_bytes = rwasm_module
            .write_binary(&mut binary_format_writer)
            .expect("failed to encode rWASM");
        assert_eq!(n_bytes, encoded_length, "encoded bytes mismatch");
        self.sdk.native_sdk().write(rwasm_bytecode);
    }
}

basic_entrypoint!(RWASM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("./greeting.wasm");
        let native_sdk = TestingContext::new().with_input(greeting_bytecode);
        let sdk = JournalState::empty(native_sdk.clone());
        let rwasm = RWASM::new(sdk);
        rwasm.deploy();
        rwasm.main();
        let output = native_sdk.output();
        let module = RwasmModule::new(&output).unwrap();
        assert!(module.code_section.len() > 0);
        assert_eq!(&module.memory_section, "Hello, World".as_bytes());
    }
}

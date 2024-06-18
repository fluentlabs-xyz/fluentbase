#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, create_sovereign_import_linker, SharedAPI};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

#[derive(Default)]
struct RWASM;

impl RWASM {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        let input_size = SDK::input_size() as usize;
        let wasm_binary = alloc_slice(input_size);
        SDK::read(wasm_binary.as_mut_ptr(), input_size as u32, 0);
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
        SDK::write(rwasm_bytecode.as_ptr(), rwasm_bytecode.len() as u32);
    }
}

basic_entrypoint!(RWASM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_works() {
        let greeting_bytecode = include_bytes!("./greeting.wasm");
        LowLevelSDK::with_test_input(greeting_bytecode.into());
        let rwasm = RWASM::default();
        rwasm.deploy::<LowLevelSDK>();
        rwasm.main::<LowLevelSDK>();
        let test_output = LowLevelSDK::get_test_output();
        let module = RwasmModule::new(&test_output).unwrap();
        assert!(module.code_section.len() > 0);
        assert_eq!(&module.memory_section, "Hello, World".as_bytes());
    }
}

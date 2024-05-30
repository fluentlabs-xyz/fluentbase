#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use core::{alloc::Layout, ptr};
use fluentbase_sdk::{create_sovereign_import_linker, LowLevelAPI, LowLevelSDK};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

// Function to deploy the contract
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn deploy() {}

// Main function
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn main() {
    let size = LowLevelSDK::sys_input_size() as usize;
    let wasm_binary = unsafe {
        let buffer = alloc::alloc::alloc(Layout::from_size_align_unchecked(size, 8usize));
        &mut *ptr::slice_from_raw_parts_mut(buffer, size)
    };
    LowLevelSDK::sys_read(wasm_binary, 0);
    let import_linker = create_sovereign_import_linker();
    let rwasm_module =
        RwasmModule::compile(wasm_binary, Some(import_linker)).expect("failed to compile");
    let encoded_length = rwasm_module.encoded_length();
    let rwasm_bytecode = unsafe {
        let buffer = alloc::alloc::alloc(Layout::from_size_align_unchecked(encoded_length, 8usize));
        &mut *ptr::slice_from_raw_parts_mut(buffer, encoded_length)
    };
    let mut binary_format_writer = BinaryFormatWriter::new(rwasm_bytecode);
    let n_bytes = rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rWASM");
    assert_eq!(n_bytes, encoded_length, "encoded bytes mismatch");
    LowLevelSDK::sys_write(rwasm_bytecode);
    LowLevelSDK::sys_halt(0);
}

#[cfg(test)]
#[test]
fn test_example_rwasm() {
    let wasm_binary = include_bytes!("./bin/rwasm.wasm");
    LowLevelSDK::with_test_input(wasm_binary.to_vec());
    main();
}

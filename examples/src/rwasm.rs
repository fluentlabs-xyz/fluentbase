use crate::deploy_internal;
use alloc::string::ToString;
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use rwasm_codegen::{rwasm::common::ValueType, Compiler, CompilerConfig, ImportFunc, ImportLinker};

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/rwasm.wasm"))
}

pub fn main() {
    let size = LowLevelSDK::sys_input_size() as usize;
    let buffer = unsafe {
        let buffer = alloc::alloc::alloc(Layout::from_size_align_unchecked(size, 8usize));
        &mut *ptr::slice_from_raw_parts_mut(buffer, size)
    };
    LowLevelSDK::sys_read(buffer, 0);
    let mut import_linker = ImportLinker::default();
    import_linker.insert_function(ImportFunc::new_env(
        "fluentbase_v1alpha".to_string(),
        "_sys_halt".to_string(),
        100,
        &[ValueType::I32],
        &[],
        0,
    ));
    import_linker.insert_function(ImportFunc::new_env(
        "fluentbase_v1alpha".to_string(),
        "_sys_write".to_string(),
        101,
        &[ValueType::I32; 2],
        &[],
        0,
    ));
    import_linker.insert_function(ImportFunc::new_env(
        "fluentbase_v1alpha".to_string(),
        "_sys_input_size".to_string(),
        102,
        &[],
        &[ValueType::I32; 1],
        0,
    ));
    import_linker.insert_function(ImportFunc::new_env(
        "fluentbase_v1alpha".to_string(),
        "_sys_read".to_string(),
        103,
        &[ValueType::I32; 3],
        &[],
        0,
    ));
    let mut compiler =
        Compiler::new_with_linker(buffer, CompilerConfig::default(), Some(&import_linker)).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    LowLevelSDK::sys_write(&rwasm_bytecode);
    LowLevelSDK::sys_halt(0);
}

#[cfg(test)]
#[test]
fn test_example_rwasm() {
    let wasm_binary = include_bytes!("../bin/rwasm.wasm");
    LowLevelSDK::with_test_input(wasm_binary.to_vec());
    main();
}

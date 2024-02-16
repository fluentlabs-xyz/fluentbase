use crate::deploy_internal;
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use rwasm_codegen::{Compiler, CompilerConfig, ImportLinker, ImportLinkerDefaults};

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
    ImportLinkerDefaults::new_v1alpha().register_import_funcs(&mut import_linker);
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

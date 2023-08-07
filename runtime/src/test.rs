use crate::runtime::Runtime;
use fluentbase_rwasm::{module::ImportName, rwasm::Compiler};
use std::collections::BTreeMap;

fn wat2rwasm(wat: &str) -> Vec<u8> {
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler = Compiler::new(&wasm_binary).unwrap();
    compiler.translate().unwrap();
    compiler.finalize().unwrap()
}

fn wasm2rwasm(wasm_binary: &[u8], host_function_mapping: Option<BTreeMap<ImportName, u32>>) -> Vec<u8> {
    let mut compiler = if let Some(host_function_mapping) = host_function_mapping {
        Compiler::new_with_linker(&wasm_binary.to_vec(), host_function_mapping)
    } else {
        Compiler::new(&wasm_binary.to_vec())
    }
    .unwrap();
    compiler.finalize().unwrap()
}

#[test]
fn test_simple() {
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (func $main
    global.get 0
    global.get 1
    call $add
    global.get 2
    call $add
    drop
    )
  (func $add (param $lhs i32) (param $rhs i32) (result i32)
    local.get $lhs
    local.get $rhs
    i32.add
    )
  (global (;0;) i32 (i32.const 100))
  (global (;1;) i32 (i32.const 20))
  (global (;2;) i32 (i32.const 3))
  (export "main" (func $main)))
    "#,
    );
    Runtime::run(rwasm_binary.as_slice(), &[]).unwrap();
}

#[test]
fn test_greeting() {
    let wasm_binary = include_bytes!("../examples/bin/greeting.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, Some(import_linker.index_mapping()));
    Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker).unwrap();
}

use crate::{evm_keccak256, runtime::Runtime, Error};
use fluentbase_rwasm::{
    common::Trap,
    rwasm::{Compiler, ImportLinker},
};

fn wat2rwasm(wat: &str) -> Vec<u8> {
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler = Compiler::new(&wasm_binary).unwrap();
    compiler.finalize().unwrap()
}

fn wasm2rwasm(wasm_binary: &[u8], import_linker: &ImportLinker) -> Vec<u8> {
    Compiler::new_with_linker(&wasm_binary.to_vec(), Some(import_linker))
        .unwrap()
        .finalize()
        .unwrap()
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
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
    let output = Runtime::run_with_linker(
        rwasm_binary.as_slice(),
        &[100, 20, 3],
        &import_linker,
        false,
    )
    .unwrap();
    assert_eq!(output.data().output().clone(), vec![0, 0, 0, 123]);
}

fn assert_trap_i32_exit<T>(result: Result<T, Error>, trap_code: Trap) {
    let err = result.err().unwrap();
    match err {
        Error::Rwasm(err) => match err {
            fluentbase_rwasm::Error::Trap(trap) => {
                assert_eq!(
                    trap.i32_exit_status().unwrap(),
                    trap_code.i32_exit_status().unwrap()
                )
            }
            _ => unreachable!("incorrect error type"),
        },
        _ => unreachable!("incorrect error type"),
    }
}

#[test]
fn test_panic() {
    let wasm_binary = include_bytes!("../examples/bin/panic.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
    let result = Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker, false);
    assert_trap_i32_exit(result, Trap::i32_exit(1));
}

#[test]
#[ignore]
fn test_translator() {
    let wasm_binary = include_bytes!("../examples/bin/rwasm.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
    let result =
        Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker, false).unwrap();
    println!("{:?}", result.data().output().clone());
}

/// EVM instructions
// use fluentbase_sdk::{evm_keccak256, sys_read};

#[test]
fn test_keccak256() {
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (import "env" "_evm_keccak256" (func $_evm_keccak256 (type 0)))
  (func $main (type 1)
    i32.const 0
    i32.const 12
    i32.const 50
    call $_evm_keccak256
    )
  (memory (;0;) 100)
  (data (;0;) (i32.const 0) "Hello, World")
  (export "main" (func $main)))
    "#,
    );
    Runtime::run(rwasm_binary.as_slice(), &[]).unwrap();

    // let wasm_binary = include_bytes!("../examples/bin/evm.wasm");
    // let import_linker = Runtime::new_linker();
    // let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);

    // // Test Data
    // let data1 = b"Hello, World!";
    // evm_keccak256(caller, offset, size);
    // let offset1 = 0;
    // let size1 = data1.len() as u32;
    // let result =
    //     Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker, false).unwrap();

    // println!("{:?}", result.data().output().clone());
}

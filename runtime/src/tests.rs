use crate::{evm_keccak256, runtime::Runtime, Error};
use fluentbase_rwasm::{
    common::Trap,
    rwasm::{Compiler, ImportLinker},
};
use tiny_keccak::{keccakf, Hasher, Sha3};

fn wat2rwasm(wat: &str, import_linker: Option<&ImportLinker>) -> Vec<u8> {
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler = Compiler::new_with_linker(&wasm_binary, import_linker).unwrap();
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
        None,
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

#[test]
fn test_keccak256() {
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32 i32)))
  (import "env" "_evm_keccak256" (func $_evm_keccak256 (type 0)))
  (import "env" "_evm_return" (func $_evm_return (type 2)))
  (func $main (type 1)
    i32.const 0
    i32.const 12
    i32.const 50
    call $_evm_keccak256
    i32.const 50
    i32.const 32
    call $_evm_return
    )
  (memory (;0;) 100)
  (data (;0;) (i32.const 0) "Hello, World")
  (export "main" (func $main)))
    "#,
        Some(&import_linker),
    );
    let result =
        Runtime::run_with_linker(rwasm_binary.as_slice(), &[], &import_linker, false).unwrap();
    let mut hasher = Sha3::v256();
    hasher.update("Hello, World".as_bytes());
    let mut expected_hash = [0u8; 32];
    hasher.finalize(&mut expected_hash);
    assert_eq!(expected_hash, result.data().output().as_slice());
}

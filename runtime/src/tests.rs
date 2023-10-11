use crate::{instruction::sys_write, runtime::Runtime, Error, HASH_SCHEME_DONE};
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

#[test]
fn zktrie_open_test() {
    use HASH_SCHEME_DONE;
    assert_eq!(*HASH_SCHEME_DONE, true);

    let wasm_binary = include_bytes!("../examples/bin/zktrie_open_test.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);

    let mut input_data = vec![];

    let root_updated: Vec<u8> = vec![
        0x23, 0x36, 0x5e, 0xbd, 0x71, 0xa7, 0xad, 0x35, 0x65, 0xdd, 0x24, 0x88, 0x47, 0xca, 0xe8,
        0xe8, 0x8, 0x21, 0x15, 0x62, 0xc6, 0x83, 0xdb, 0x8, 0x4f, 0x5a, 0xfb, 0xd1, 0xb0, 0x3d,
        0x4c, 0xb5,
    ];
    input_data.extend(root_updated);

    let key = "key".as_bytes();
    let mut key_bytes = [0u8; 20];
    let l = key_bytes.len();
    key_bytes[l - key.len()..].copy_from_slice(key);
    input_data.extend(key_bytes.as_slice());

    let mut account_data = [0u8; 32 * 5];
    account_data[0] = 1;
    input_data.extend(account_data.as_slice());

    println!("input_data: {:?} len {}", input_data, input_data.len());

    let output =
        Runtime::run_with_linker(rwasm_binary.as_slice(), &input_data, &import_linker, false)
            .unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
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

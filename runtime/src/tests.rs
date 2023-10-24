use crate::{runtime::Runtime, RuntimeContext, RuntimeError, SysFuncIdx, HASH_SCHEME_DONE};
use fluentbase_rwasm::{
    common::Trap,
    engine::bytecode::Instruction,
    rwasm::{Compiler, FuncOrExport, ImportLinker},
};

fn wat2rwasm(wat: &str) -> Vec<u8> {
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler = Compiler::new(&wasm_binary).unwrap();
    compiler.finalize().unwrap()
}

fn wasm2rwasm(wasm_binary: &[u8]) -> Vec<u8> {
    let import_linker = Runtime::new_linker();
    Compiler::new_with_linker(&wasm_binary.to_vec(), Some(&import_linker))
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
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let output = Runtime::run(rwasm_binary.as_slice(), &[100, 20, 3]).unwrap();
    assert_eq!(output.data().output().clone(), vec![0, 0, 0, 123]);
}

#[test]
fn zktrie_open_test() {
    use HASH_SCHEME_DONE;
    assert_eq!(*HASH_SCHEME_DONE, true);

    let wasm_binary = include_bytes!("../examples/bin/zktrie_open_test.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    let input_data = vec![];

    let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
}

#[test]
fn mpt_open_test() {
    let wasm_binary = include_bytes!("../examples/bin/mpt_open_test.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    let input_data = [];

    let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
}

#[test]
fn keccak_test() {
    let wasm_binary = include_bytes!("../examples/bin/crypto_keccak.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    let input_data: &[u8] = "hello world".as_bytes();

    let output = Runtime::run(rwasm_binary.as_slice(), input_data).unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
}

#[test]
fn poseidon_test() {
    let wasm_binary =
        wat::parse_bytes(include_bytes!("../examples/bin/crypto_poseidon.wat")).unwrap();
    let rwasm_binary = wasm2rwasm(&wasm_binary);

    let input_data: &[u8] = "hello world".as_bytes();

    let output = Runtime::run(rwasm_binary.as_slice(), input_data).unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
}

fn assert_trap_i32_exit<T>(result: Result<T, RuntimeError>, trap_code: Trap) {
    let err = result.err().unwrap();
    match err {
        RuntimeError::Rwasm(err) => match err {
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
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let result = Runtime::run(rwasm_binary.as_slice(), &[]);
    assert_trap_i32_exit(result, Trap::i32_exit(71));
}

#[test]
#[ignore]
fn test_translator() {
    let wasm_binary = include_bytes!("../examples/bin/rwasm.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let result = Runtime::run(rwasm_binary.as_slice(), &[]).unwrap();
    println!("{:?}", result.data().output().clone());
}

#[test]
fn test_state() {
    let wasm_binary = wat::parse_str(
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
  (func $deploy
    )
  (func $add (param $lhs i32) (param $rhs i32) (result i32)
    local.get $lhs
    local.get $rhs
    i32.add
    )
  (global (;0;) i32 (i32.const 100))
  (global (;1;) i32 (i32.const 20))
  (global (;2;) i32 (i32.const 3))
  (export "main" (func $main))
  (export "deploy" (func $deploy)))
    "#,
    )
    .unwrap();
    let import_linker = Runtime::new_linker();
    let mut compiler =
        Compiler::new_with_linker(wasm_binary.as_slice(), Some(&import_linker)).unwrap();
    compiler
        .translate(Some(FuncOrExport::StateRouter(
            vec![FuncOrExport::Export("main"), FuncOrExport::Export("deploy")],
            Instruction::Call((SysFuncIdx::SYS_STATE as u32).into()),
        )))
        .unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    Runtime::run_with_context(RuntimeContext::new(rwasm_bytecode), &import_linker).unwrap();
}

use crate::{runtime::Runtime, Error, RuntimeContext, SysFuncIdx, HASH_SCHEME_DONE, runtime};
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
    let output = Runtime::run_with_input(
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
        1, 158, 59, 182, 29, 224, 81, 156, 63, 5, 24, 82, 92, 243, 23, 118, 114, 252, 249, 133, 70,
        229, 137, 214, 108, 4, 219, 78, 152, 25, 152, 109,
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

    let output =
        Runtime::run_with_input(rwasm_binary.as_slice(), &input_data, &import_linker, false)
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
    let result = Runtime::run_with_input(rwasm_binary.as_slice(), &[], &import_linker, false);
    assert_trap_i32_exit(result, Trap::i32_exit(1));
}

#[test]
#[ignore]
fn test_translator() {
    let wasm_binary = include_bytes!("../examples/bin/rwasm.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
    let result =
        Runtime::run_with_input(rwasm_binary.as_slice(), &[], &import_linker, false).unwrap();
    println!("{:?}", result.data().output().clone());
}

#[test]
fn test_import_section() {
    let wasm_binary = include_bytes!("../examples/bin/import.wasm");
    let import_linker = Runtime::new_linker();
    let rwasm_binary = wasm2rwasm(wasm_binary, &import_linker);
    let result =
        Runtime::run_with_input(rwasm_binary.as_slice(), &[], &import_linker, false).unwrap();
    println!("Output: {:?}", result.data().output().clone());
}

#[test]
fn test_state() {
    let wasm_binary = wat::parse_str(
        r#"
(module
  (memory 1)
  (data (i32.const 0) "\01\00\00\00\00\00\00\00")

  (func $main
    i32.const 0
    i32.load
    drop
    )
  (func $add (param $lhs i32) (param $rhs i32) (result i32)
    local.get $lhs
    local.get $rhs
    i32.add
    )
  (func $deploy
    i32.const 1
    memory.grow
    drop
    i32.const 0
    i32.const 1000
    i32.store
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
        .translate_with_state(Some(FuncOrExport::StateRouter(
            vec![FuncOrExport::Export("main"), FuncOrExport::Export("deploy")],
            Instruction::Call((SysFuncIdx::SYS_STATE as u32).into()),
        )), true)
        .unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();

    let mut runtime = Runtime::new_with_context(
        rwasm_bytecode.as_slice(),
        RuntimeContext::new(&[], 1000),
        &import_linker,
    )
        .unwrap();

    let pre = runtime.init_pre_instance().unwrap();

    let start_func =pre.get_start_func(&mut runtime.store).unwrap();

    start_func.call(&mut runtime.store,&[], &mut []).unwrap();

    runtime.set_state(0);

    start_func.call(&mut runtime.store,&[], &mut []).unwrap();

    runtime.set_state(1);

    start_func.call(&mut runtime.store,&[], &mut []).unwrap();

    runtime.set_state(0);

    start_func.call(&mut runtime.store,&[], &mut []).unwrap();

}

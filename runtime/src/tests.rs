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

#[cfg(test)]
mod ttt {
    use crate::{tests::wat2rwasm, Runtime};

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
    let wasm_binary = include_bytes!("../examples/bin/crypto_poseidon.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    let input_data: &[u8] = "hello world".as_bytes();

    let output = Runtime::run(rwasm_binary.as_slice(), input_data).unwrap();
    assert_eq!(output.data().output().clone(), vec![]);
}

#[test]
fn secp256k1_verify_test() {
    let wasm_binary = include_bytes!("../examples/bin/secp256k1_verify.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    // RecoveryTestVector {
    //             pk: hex!("021a7a569e91dbf60581509c7fc946d1003b60c7dee85299538db6353538d59574"),
    //             msg: b"example message",
    //             sig: hex!(
    //
    // "ce53abb3721bafc561408ce8ff99c909f7f0b18a2f788649d6470162ab1aa0323971edc523a6d6453f3fb6128d318d9db1a5ff3386feb1047d9816e780039d52"
    //             ),
    //             recid: RecoveryId::new(false, false),
    //             recid2: 0,
    //         },
    //         // Recovery ID 1
    //         RecoveryTestVector {
    //             pk: hex!("036d6caac248af96f6afa7f904f550253a0f3ef3f5aa2fe6838a95b216691468e2"),
    //             msg: b"example message",
    //             sig: hex!(
    //
    // "46c05b6368a44b8810d79859441d819b8e7cdc8bfd371e35c53196f4bcacdb5135c7facce2a97b95eacba8a586d87b7958aaf8368ab29cee481f76e871dbd9cb"
    //             ),
    //             recid: RecoveryId::new(true, false),
    //             recid2: 1,
    //         },
    let mut input_data: &[u8] = &[
        173, 132, 205, 11, 16, 252, 2, 135, 56, 151, 27, 7, 129, 36, 174, 194, 160, 231, 198, 217,
        134, 163, 129, 190, 11, 56, 111, 50, 190, 232, 135, 175, 206, 83, 171, 179, 114, 27, 175,
        197, 97, 64, 140, 232, 255, 153, 201, 9, 247, 240, 177, 138, 47, 120, 134, 73, 214, 71, 1,
        98, 171, 26, 160, 50, 57, 113, 237, 197, 35, 166, 214, 69, 63, 63, 182, 18, 141, 49, 141,
        157, 177, 165, 255, 51, 134, 254, 177, 4, 125, 152, 22, 231, 128, 3, 157, 82, 0, 2, 26,
        122, 86, 158, 145, 219, 246, 5, 129, 80, 156, 127, 201, 70, 209, 0, 59, 96, 199, 222, 232,
        82, 153, 83, 141, 182, 53, 53, 56, 213, 149, 116,
    ];

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

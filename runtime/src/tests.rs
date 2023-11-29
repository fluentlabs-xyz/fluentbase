use crate::{runtime::Runtime, RuntimeContext, RuntimeError, SysFuncIdx};
use eth_trie::DB;
use fluentbase_rwasm::{
    engine::bytecode::Instruction,
    rwasm::{Compiler, FuncOrExport, ReducedModule},
};
use fluentbase_rwasm_core::common::Trap;
use keccak_hash::H256;
use serde_json::from_str;
use std::{borrow::BorrowMut, cell::RefMut, env, fs::File, io::Read, rc::Rc, sync::Arc};

pub(crate) fn wat2rwasm(wat: &str, consume_fuel: bool) -> Vec<u8> {
    let import_linker = Runtime::<()>::new_linker();
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler =
        Compiler::new_with_linker(&wasm_binary, Some(&import_linker), consume_fuel).unwrap();
    compiler.finalize().unwrap()
}

fn wasm2rwasm(wasm_binary: &[u8], inject_fuel_consumption: bool) -> Vec<u8> {
    let import_linker = Runtime::<()>::new_linker();
    Compiler::new_with_linker(
        &wasm_binary.to_vec(),
        Some(&import_linker),
        inject_fuel_consumption,
    )
    .unwrap()
    .finalize()
    .unwrap()
}

#[cfg(test)]
mod tests {
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
            true,
        );
        Runtime::<()>::run(rwasm_binary.as_slice(), &Vec::new(), 10_000_000).unwrap();
    }
}

#[test]
fn test_greeting() {
    let wasm_binary = include_bytes!("../../examples/bin/greeting.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary, true);
    let output = Runtime::<()>::run(rwasm_binary.as_slice(), &vec![], 10_000_000).unwrap();
    assert_eq!(
        output.data().output().clone(),
        "Hello, World".as_bytes().to_vec()
    );
}

// #[test]
// fn zktrie_open_test() {
//     use HASH_SCHEME_DONE;
//     assert_eq!(*HASH_SCHEME_DONE, true);
//
//     let wasm_binary = include_bytes!("../../examples/bin/zktrie_open_test.wasm");
//     let rwasm_binary = wasm2rwasm(wasm_binary);
//
//     let input_data = vec![];
//
//     let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
//     assert_eq!(output.data().output().clone(), vec![]);
// }
//
// #[test]
// fn mpt_open_test() {
//     let wasm_binary = include_bytes!("../../examples/bin/mpt_open_test.wasm");
//     let rwasm_binary = wasm2rwasm(wasm_binary);
//
//     let input_data = [];
//
//     let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
//     assert_eq!(output.data().output().clone(), vec![]);
// }

#[test]
fn test_keccak256_example() {
    let wasm_binary = include_bytes!("../../examples/bin/keccak256.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary, true);

    let input_data: &[u8] = "hello world".as_bytes();
    let output =
        Runtime::<()>::run(rwasm_binary.as_slice(), &input_data.to_vec(), 10_000_000).unwrap();
    assert_eq!(output.data().output().clone(), Vec::<u8>::new());
}

#[test]
fn test_poseidon() {
    let wasm_binary = include_bytes!("../../examples/bin/poseidon.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary, true);

    let input_data: &[u8] = "hello world".as_bytes();

    let output =
        Runtime::<()>::run(rwasm_binary.as_slice(), &input_data.to_vec(), 10_000_000).unwrap();
    assert_eq!(output.data().output().clone(), Vec::<u8>::new());
}

#[test]
fn test_secp256k1_verify() {
    let wasm_binary = include_bytes!("../../examples/bin/secp256k1.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary, true);

    let input_datas: &[&[u8]] = &[
        &[
            173, 132, 205, 11, 16, 252, 2, 135, 56, 151, 27, 7, 129, 36, 174, 194, 160, 231, 198,
            217, 134, 163, 129, 190, 11, 56, 111, 50, 190, 232, 135, 175, 206, 83, 171, 179, 114,
            27, 175, 197, 97, 64, 140, 232, 255, 153, 201, 9, 247, 240, 177, 138, 47, 120, 134, 73,
            214, 71, 1, 98, 171, 26, 160, 50, 57, 113, 237, 197, 35, 166, 214, 69, 63, 63, 182, 18,
            141, 49, 141, 157, 177, 165, 255, 51, 134, 254, 177, 4, 125, 152, 22, 231, 128, 3, 157,
            82, 0, 2, 26, 122, 86, 158, 145, 219, 246, 5, 129, 80, 156, 127, 201, 70, 209, 0, 59,
            96, 199, 222, 232, 82, 153, 83, 141, 182, 53, 53, 56, 213, 149, 116,
        ],
        &[
            173, 132, 205, 11, 16, 252, 2, 135, 56, 151, 27, 7, 129, 36, 174, 194, 160, 231, 198,
            217, 134, 163, 129, 190, 11, 56, 111, 50, 190, 232, 135, 175, 70, 192, 91, 99, 104,
            164, 75, 136, 16, 215, 152, 89, 68, 29, 129, 155, 142, 124, 220, 139, 253, 55, 30, 53,
            197, 49, 150, 244, 188, 172, 219, 81, 53, 199, 250, 204, 226, 169, 123, 149, 234, 203,
            168, 165, 134, 216, 123, 121, 88, 170, 248, 54, 138, 178, 156, 238, 72, 31, 118, 232,
            113, 219, 217, 203, 1, 3, 109, 108, 170, 194, 72, 175, 150, 246, 175, 167, 249, 4, 245,
            80, 37, 58, 15, 62, 243, 245, 170, 47, 230, 131, 138, 149, 178, 22, 105, 20, 104, 226,
        ],
    ];

    for input_data in input_datas {
        let output =
            Runtime::<()>::run(rwasm_binary.as_slice(), &input_data.to_vec(), 10_000_000).unwrap();
        assert_eq!(output.data().output().clone(), Vec::<u8>::new());
    }
}

#[test]
fn test_panic() {
    let wasm_binary = include_bytes!("../../examples/bin/panic.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary, true);
    let result = Runtime::<()>::run(rwasm_binary.as_slice(), &Vec::new(), 10_000_000).unwrap();
    assert_eq!(result.data().exit_code(), 71);
}

// #[test]
// #[ignore]
// fn test_translator() {
//     let wasm_binary = include_bytes!("../../examples/bin/rwasm.wasm");
//     let rwasm_binary = wasm2rwasm(wasm_binary);
//     let result = Runtime::run(rwasm_binary.as_slice(), &Vec::new()).unwrap();
//     println!("{:?}", result.data().output().clone());
// }

#[test]
fn rwasm_compile_with_linker_test() {
    let wasm_binary_to_execute =
        include_bytes!("../../examples/bin/rwasm_compile_with_linker_test.wasm");
    let rwasm_binary_to_execute = wasm2rwasm(wasm_binary_to_execute, true);
    let wasm_binary_to_compile = include_bytes!("../../examples/bin/greeting.wasm");
    // let rwasm_binary_compile_res_len = wasm2rwasm(wasm_binary_to_compile);
    // println!("wasm_binary_to_compile {}", wasm_binary_to_compile.len());
    // println!(
    //     "rwasm_binary_compile_res_len {}",
    //     rwasm_binary_compile_res_len.len()
    // );
    let input = wasm_binary_to_compile.to_vec();
    let result =
        Runtime::<()>::run(rwasm_binary_to_execute.as_slice(), &input, 10_000_000).unwrap();
    println!("{:?}", result.data().output().clone());
    assert_eq!(result.data().output().clone(), Vec::<u8>::new());
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
    let import_linker = Runtime::<()>::new_linker();
    let mut compiler =
        Compiler::new_with_linker(wasm_binary.as_slice(), Some(&import_linker), true).unwrap();
    compiler
        .translate(
            Some(FuncOrExport::StateRouter(
                vec![FuncOrExport::Export("main"), FuncOrExport::Export("deploy")],
                Instruction::Call((SysFuncIdx::SYS_STATE as u32).into()),
            )),
            true,
        )
        .unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    Runtime::<()>::run_with_context(RuntimeContext::new(rwasm_bytecode), &import_linker).unwrap();
}

#[test]
fn test_keccak256() {
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32 i32)))
  (import "env" "_crypto_keccak256" (func $_evm_keccak256 (type 0)))
  (import "env" "_sys_write" (func $_evm_return (type 2)))
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
        false,
    );

    let mut rmodule = ReducedModule::new(&rwasm_binary, false).unwrap();
    println!("rmodule.trace_binary(): {:?}", rmodule.trace());
    let execution_result = Runtime::<()>::run(rwasm_binary.as_slice(), &Vec::new(), 0).unwrap();
    println!(
        "execution_result (exit_code {}): {:?}",
        execution_result.data().exit_code,
        execution_result
    );
    match hex::decode("0xa04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529") {
        Ok(answer) => {
            assert_eq!(&answer, execution_result.data().output().as_slice());
        }
        Err(e) => {
            // If there's an error, you might want to handle it in some way.
            // For this example, I'll just print the error.
            println!("Error: {:?}", e);
        }
    }
}

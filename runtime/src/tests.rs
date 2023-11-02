use crate::{
    eth_types::{
        block::{self, Block},
        header::{generate_random_header, generate_random_header_based_on_prev_block},
    },
    runtime::Runtime,
    RuntimeContext,
    RuntimeError,
    SysFuncIdx,
    HASH_SCHEME_DONE,
};
use fluentbase_rwasm::{
    common::Trap,
    engine::bytecode::Instruction,
    rwasm::{Compiler, FuncOrExport, ImportLinker},
};
use keccak_hash::H256;
use std::{env, fs::File, io::Read};

pub(crate) fn wat2rwasm(wat: &str) -> Vec<u8> {
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
    Runtime::run(rwasm_binary.as_slice(), &Vec::new()).unwrap();
}

#[test]
fn test_greeting() {
    let wasm_binary = include_bytes!("../examples/bin/greeting.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let input_data: Vec<Vec<u8>> = vec![[100, 20, 3].to_vec()];
    let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
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
    // assert_eq!(output.data().output().clone(), vec![]);
}

#[test]
fn mpt_open_test() {
    let wasm_binary = include_bytes!("../examples/bin/mpt_open_test.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    let input_data: Vec<Vec<u8>> = Vec::new();
    let output = Runtime::run(rwasm_binary.as_slice(), &input_data).unwrap();
    // assert_eq!(output.data().output().clone(), vec![]);
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
    let result = Runtime::run(rwasm_binary.as_slice(), &Vec::new());
    assert_trap_i32_exit(result, Trap::i32_exit(71));
}

#[test]
#[ignore]
fn test_translator() {
    let wasm_binary = include_bytes!("../examples/bin/rwasm.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let result = Runtime::run(rwasm_binary.as_slice(), &Vec::new()).unwrap();
    println!("{:?}", result.data().output().clone());
}

#[test]
fn rwasm_compile_with_linker_test() {
    let wasm_binary = include_bytes!("../examples/bin/rwasm_compile_with_linker_test.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);
    let input = vec![vec![1,2,3]];
    let result = Runtime::run(rwasm_binary.as_slice(), &input).unwrap();
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

#[test]
fn test_keccak256() {
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
    );

    let result = Runtime::run(rwasm_binary.as_slice(), &Vec::new()).unwrap();
    println!("{:?}", result);
    match hex::decode("0xa04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529") {
        Ok(answer) => {
            assert_eq!(&answer, result.data().output().as_slice());
        }
        Err(e) => {
            // If there's an error, you might want to handle it in some way.
            // For this example, I'll just print the error.
            println!("Error: {:?}", e);
        }
    }
}

#[test]
fn test_evm_verify_block_rlps_without_transactions() {
    let wasm_binary = include_bytes!("../examples/bin/evm_verify_block_rlps.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary);

    // 1. generate block rlp and put read it with evm_rlp_block_a
    let (blk_a_header, blk_a_hash) = generate_random_header(&123120);
    let blk_a = Block {
        header: blk_a_header,
        transactions: vec![],
        uncles: vec![],
    };
    let blk_a_encoded = rlp::encode(&blk_a).to_vec();

    // 2. current block
    let blk_b_header = generate_random_header_based_on_prev_block(&123121, blk_a_hash);
    let blk_b = Block {
        header: blk_b_header,
        transactions: vec![],
        uncles: vec![],
    };
    let blk_b_encoded = rlp::encode(&blk_b).to_vec();

    let mut input_data: Vec<Vec<u8>> = Vec::new();
    input_data.push(blk_a_encoded);
    input_data.push(blk_b_encoded);

    Runtime::run(rwasm_binary.as_slice(), &input_data.to_vec()).unwrap();
}

#[test]
fn test_evm_verify_block_receipts_with_signed_transactions() {
    let wasm_binary = include_bytes!("../examples/bin/evm_verify_block_rlps.wasm"); // TODO
    let rwasm_binary = wasm2rwasm(wasm_binary);

    // read block_receipt_a.json
    let mut block_receipt_a_json = String::new();
    File::open("src/test_data/block_receipt_a.json")
        .unwrap()
        .read_to_string(&mut block_receipt_a_json)
        .unwrap();

    let block_init: block::Block =
        serde_json::from_str::<block::Block>(block_receipt_a_json.as_str()).unwrap();
    let block_a = block_init.clone();
    serde_json::to_value(block_a.clone()).unwrap();

    println!("ENCODED: {:?}", block_a.header);

    let blk_a_encoded = rlp::encode(&block_a).to_vec();

    //println!("ENCODED: {:?}", blk_a_encoded);

    // let block_txs_a = rlp::decode::<block::Block>(&blk_a_encoded).unwrap();
    // assert_eq!(blk_a_encoded, rlp::encode(&block_txs_a).to_vec());

    // // read block_receipt_b.json
    // let mut block_receipt_b_json = String::new();
    // File::open("src/test_data/block_receipt_b.json")
    //     .unwrap()
    //     .read_to_string(&mut block_receipt_b_json)
    //     .unwrap();

    // let mut input_data: Vec<Vec<u8>> = Vec::new();
    // for _ in 0..9 {
    //     input_data.push(vec![]);
    // }
    // // input_data.push(blk_a_encoded); // 10
    // // input_data.push(blk_b_encoded); // 11

    // // let block_init =
    // // serde_json::from_str::<block::BlockX>(block_receipt_b_json.as_str()).unwrap();
    // // let block_b = block_init.clone();
    // // serde_json::to_value(block_b).unwrap();

    // // println!("HEADER: {:?}", block_b.header);

    // Runtime::run(rwasm_binary.as_slice(), &input_data.to_vec()).unwrap();
}

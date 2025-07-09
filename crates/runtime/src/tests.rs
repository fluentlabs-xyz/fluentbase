use crate::{runtime::Runtime, RuntimeContext};
use fluentbase_types::{
    compile_wasm_to_rwasm,
    keccak256,
    Address,
    BytecodeOrHash,
    Bytes,
    STATE_DEPLOY,
    STATE_MAIN,
};
use hex_literal::hex;
use rwasm::Store;

pub(crate) fn wat2rwasm(wat: &str) -> Bytes {
    let wasm_binary = wat::parse_str(wat).unwrap();
    let result = compile_wasm_to_rwasm(&wasm_binary).unwrap();
    result.rwasm_module.serialize().into()
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

    let ctx = RuntimeContext::new(new_bytecode_or_hash(rwasm_binary)).with_fuel_limit(10_000_000);
    let execution_result = Runtime::run_with_context(ctx);
    assert_eq!(execution_result.exit_code, 0);
}

#[test]
fn test_wrong_indirect_type() {
    let rwasm_bytecode = wat2rwasm(
        r#"
(module
    (type $right (func (param i32) (result i32)))
    (type $wrong (func (param i64) (result i64)))
    (func $const-i32 (type $right) (local.get 0))
    (func $id-i64 (type $wrong) (local.get 0))
    (table funcref
        (elem
          $const-i32 $id-i64
        )
    )
    (func (export "deploy"))
    (func (export "main")
        (i64.const 0)
        (call_indirect (type $wrong) (i32.const 0xffffffff))
        (drop)
    ))
    "#,
    );
    let ctx = RuntimeContext::new(new_bytecode_or_hash(rwasm_bytecode))
        .with_fuel_limit(1_000_000)
        .with_state(STATE_DEPLOY);
    let mut runtime = Runtime::new(ctx);
    let res = runtime.call();
    let ctx = runtime.store.context(|ctx| ctx.clone());
    assert_eq!(res.exit_code, 0);
    let res = Runtime::run_with_context(ctx.with_state(STATE_MAIN));
    assert_eq!(res.exit_code, -2003);
}

#[test]
fn test_keccak256() {
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32 i32)))
  (import "fluentbase_v1preview" "_keccak256" (func $_evm_keccak256 (type 0)))
  (import "fluentbase_v1preview" "_write" (func $_evm_return (type 2)))
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
    let ctx = RuntimeContext::new(new_bytecode_or_hash(rwasm_binary)).with_fuel_limit(1_000_000);
    let execution_result = Runtime::run_with_context(ctx);
    println!("fuel consumed: {}", execution_result.fuel_consumed);
    assert_eq!(execution_result.exit_code, 0);
    assert_eq!(
        hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529"),
        execution_result.output.as_slice()
    );
}

fn new_bytecode_or_hash(bytecode: Bytes) -> BytecodeOrHash {
    let code_hash = keccak256(bytecode.as_ref());
    BytecodeOrHash::from((Address::ZERO, bytecode, code_hash))
}

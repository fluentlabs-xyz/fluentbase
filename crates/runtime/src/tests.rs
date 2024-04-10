use crate::{runtime::Runtime, DefaultEmptyRuntimeDatabase, RuntimeContext};
use hex_literal::hex;
use rwasm::rwasm::{BinaryFormat, RwasmModule};

pub(crate) fn wat2rwasm(wat: &str) -> Vec<u8> {
    let import_linker = Runtime::<DefaultEmptyRuntimeDatabase>::new_sovereign_linker();
    let wasm_binary = wat::parse_str(wat).unwrap();
    let rwasm_module = RwasmModule::compile(&wasm_binary, Some(import_linker)).unwrap();
    let mut result = Vec::new();
    rwasm_module.write_binary_to_vec(&mut result).unwrap();
    result
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
    let ctx = RuntimeContext::new(rwasm_binary).with_fuel_limit(10_000_000);
    let import_linker = Runtime::<DefaultEmptyRuntimeDatabase>::new_sovereign_linker();
    Runtime::<DefaultEmptyRuntimeDatabase>::run_with_context(ctx, import_linker).unwrap();
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
    (func (export "main")
        (call_indirect (type $wrong) (i64.const 0xffffffffff) (i32.const 0))
        (drop)
    ))
    "#,
    );
    let import_linker = Runtime::<DefaultEmptyRuntimeDatabase>::new_sovereign_linker();
    let ctx = RuntimeContext::new(rwasm_bytecode)
        .with_fuel_limit(1_000_000)
        .with_state(1000);
    let mut runtime = Runtime::<DefaultEmptyRuntimeDatabase>::new(ctx, import_linker).unwrap();
    runtime.call().unwrap();
    runtime.data_mut().state = 0;
    let res = runtime.call();
    assert_eq!(-2008, res.as_ref().unwrap().data().exit_code());
}

#[test]
fn test_keccak256() {
    let rwasm_binary = wat2rwasm(
        r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func))
  (type (;2;) (func (param i32 i32)))
  (import "fluentbase_v1alpha" "_crypto_keccak256" (func $_evm_keccak256 (type 0)))
  (import "fluentbase_v1alpha" "_sys_write" (func $_evm_return (type 2)))
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
    let ctx = RuntimeContext::new(rwasm_binary).with_fuel_limit(1_000_000);
    let import_linker = Runtime::<DefaultEmptyRuntimeDatabase>::new_sovereign_linker();
    let execution_result =
        Runtime::<DefaultEmptyRuntimeDatabase>::run_with_context(ctx, import_linker).unwrap();
    println!(
        "fuel consumed: {}",
        execution_result.fuel_consumed().unwrap_or_default()
    );
    assert_eq!(execution_result.data().exit_code, 0);
    assert_eq!(
        hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529"),
        execution_result.data().output.as_slice()
    );
}

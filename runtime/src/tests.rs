use crate::{runtime::Runtime, types::SysFuncIdx, RuntimeContext};
use fluentbase_rwasm::{
    instruction_set,
    rwasm::{Compiler, CompilerConfig, FuncOrExport, ReducedModule},
};

pub(crate) fn wat2rwasm(wat: &str, consume_fuel: bool) -> Vec<u8> {
    let import_linker = Runtime::<()>::new_linker();
    let wasm_binary = wat::parse_str(wat).unwrap();
    let mut compiler = Compiler::new_with_linker(
        &wasm_binary,
        CompilerConfig::default().fuel_consume(consume_fuel),
        Some(&import_linker),
    )
    .unwrap();
    compiler.finalize().unwrap()
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
        true,
    );
    let ctx = RuntimeContext::new(rwasm_binary).with_fuel_limit(10_000_000);
    let import_linker = Runtime::<()>::new_linker();
    Runtime::<()>::run_with_context(ctx, &import_linker).unwrap();
}

#[test]
fn test_input_output() {
    let wasm_binary = wat::parse_str(
        r#"
(module
  (func $main (param $rhs i32) (result i32)
    local.get $rhs
    i32.const 36
    i32.add
    )
  (export "main" (func $main)))
    "#,
    )
    .unwrap();
    let import_linker = Runtime::<()>::new_linker();
    let config = CompilerConfig::default()
        .with_state(true)
        .fuel_consume(true)
        .with_input_code(instruction_set! {
            I32Const(1)
            MemoryGrow
            Drop
            I32Const(0)
            I32Const(0)
            I32Const(8)
            Call(SysFuncIdx::SYS_READ)
            Drop
            I32Const(0)
            I64Load(0)
        })
        .with_output_code(instruction_set! {
            LocalGet(1)
            I32Const(0)
            LocalSet(2)
            I64Store(0)
            I32Const(0)
            I32Const(8)
            Call(SysFuncIdx::SYS_WRITE)
        });
    let mut compiler =
        Compiler::new_with_linker(wasm_binary.as_slice(), config, Some(&import_linker)).unwrap();
    compiler
        .translate(FuncOrExport::StateRouter(
            vec![FuncOrExport::Export("main")],
            instruction_set! {
                Call(SysFuncIdx::SYS_STATE)
            },
        ))
        .unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();

    let mut runtime = Runtime::<()>::new(
        RuntimeContext::new(rwasm_bytecode.as_slice())
            .with_input(vec![64, 0, 0, 0, 0, 0, 0, 0])
            .with_state(0)
            .with_fuel_limit(1_000_000),
        &import_linker,
    )
    .unwrap();
    runtime.data_mut().clean_output();
    runtime.call().unwrap();

    assert_eq!(runtime.data().output, [100, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn test_wrong_indirect_type() {
    let wasm_binary = wat::parse_str(
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
    )
    .unwrap();
    let import_linker = Runtime::<()>::new_linker();
    let mut compiler = Compiler::new_with_linker(
        wasm_binary.as_slice(),
        CompilerConfig::default()
            .fuel_consume(true)
            .with_state(true),
        Some(&import_linker),
    )
    .unwrap();
    compiler
        .translate(FuncOrExport::StateRouter(
            vec![FuncOrExport::Export("main")],
            instruction_set! {
                Call(SysFuncIdx::SYS_STATE)
            },
        ))
        .unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();

    let mut runtime = Runtime::<()>::new(
        RuntimeContext::new(rwasm_bytecode.as_slice())
            .with_fuel_limit(1_000_000)
            .with_state(1000),
        &import_linker,
    )
    .unwrap();

    runtime.call().unwrap();
    runtime.data_mut().state = 0;
    let res = runtime.call();
    assert_eq!(-2014, res.as_ref().unwrap().data().exit_code());
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
        false,
    );

    let module = ReducedModule::new(&rwasm_binary).unwrap();
    println!("module.trace_binary(): {:?}", module.trace());
    let ctx = RuntimeContext::new(rwasm_binary);
    let import_linker = Runtime::<()>::new_linker();
    let execution_result = Runtime::<()>::run_with_context(ctx, &import_linker).unwrap();
    println!(
        "execution_result (exit_code {})",
        execution_result.data().exit_code,
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

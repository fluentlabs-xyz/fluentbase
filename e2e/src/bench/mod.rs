use crate::helpers::run_with_default_context;
use alloy_rlp::{Bytes, Encodable};
use core::str::from_utf8;
use hex_literal::hex;
use rwasm::{engine::RwasmConfig, rwasm::RwasmModule, Config, Engine, Module};
use serde_json::Value::String;

#[test]
#[ignore]
fn test_example_keccak_rwasm() {
    let input_data = include_bytes!("../../../examples/keccak/lib.wasm");

    let input = "Hello World";

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_greeting_rwasm() {
    let input_data = include_bytes!("../../../examples/greeting/lib.wasm");

    let input = "Hello World";

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    println!("Compile and Run");
    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) =
        run_with_default_context(input_data.to_vec(), &input.to_string().into_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_panic_rwasm() {
    println!("Only compile");
    let input_data = include_bytes!("../../../examples/panic/lib.wasm");
    let mut input_bytes = vec![0, 0];
    input_bytes.append(&mut input_data.to_vec());

    let (_, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let input_data = include_bytes!("../../../examples/panic/lib.wasm");

    let mut input_bytes = vec![1, 0];
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, -71);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, -71);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_router_rwasm() {
    let input_data = include_bytes!("../../../examples/router/lib.wasm");

    let mut config = Config::default();

    config
        .wasm_mutable_global(true)
        .wasm_saturating_float_to_int(true)
        .wasm_sign_extension(true)
        .wasm_multi_value(true)
        .wasm_bulk_memory(true)
        .wasm_reference_types(true)
        .wasm_tail_call(true)
        .wasm_extended_const(true);
    config.rwasm_config(RwasmConfig {
        state_router: None,
        entrypoint_name: None,
        import_linker: None,
        wrap_import_functions: false,
    });

    let engine = Engine::new(&config);
    let original_engine = &engine;
    let original_module = Module::new(original_engine, &input_data[..]).unwrap();
    let imports = original_module.imports().collect::<Vec<_>>();

    let rwasm_module = RwasmModule::from_module(&original_module);

    println!("Imports: {:?} {:?}", original_module.imports, imports);

    let mut input = hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000").to_vec();

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.clone());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.clone());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_slice());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_chess_rwasm() {
    println!("Only compile");
    let input_data = include_bytes!("../../../examples/shakmaty/lib.wasm");
    let is_run = 0;
    let mut input_bytes = vec![is_run, 0];
    input_bytes.append(&mut input_data.to_vec());
    let (_, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let input_data = include_bytes!("../../../examples/shakmaty/lib.wasm");
    let is_run = 1;
    let mut input_bytes = vec![is_run, 0];
    input_bytes.append(&mut input_data.to_vec());
    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

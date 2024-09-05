use crate::helpers::run_with_default_context;
use core::str::from_utf8;
use hex_literal::hex;
use rwasm::{engine::RwasmConfig, rwasm::RwasmModule, Config, Engine, Module};

#[test]
fn test_example_greeting() {
    let (output, exit_code) = run_with_default_context(
        include_bytes!("../../examples/greeting/lib.wasm").to_vec(),
        "Hello, World".as_bytes(),
    );
    assert_eq!(exit_code, 0);
    assert_eq!(output.clone(), "Hello, World".as_bytes().to_vec());
}

#[test]
fn test_example_keccak256() {
    let (output, exit_code) = run_with_default_context(
        include_bytes!("../../examples/hashing/lib.wasm").to_vec(),
        "Hello, World".as_bytes(),
    );
    assert_eq!(exit_code, 0);
    assert_eq!(
        output[0..32],
        hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529").to_vec()
    );
}

#[test]
#[ignore]
fn test_example_rwasm() {
    let input_data = include_bytes!("../../examples/greeting/lib.wasm");
    let (output, exit_code) = run_with_default_context(
        include_bytes!("../../examples/rwasm/lib.wasm").to_vec(),
        input_data,
    );
    assert_eq!(exit_code, 0);
    assert_eq!(output[0], 0xef);
}

#[test]
#[ignore]
fn test_example_rwasm_jit() {
    let input_data = include_bytes!("../../examples/greeting/lib.wasm");

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
    let rwasm_module = RwasmModule::from_module(&original_module);

    println!("Imports: {:?}", original_module.imports);

    let (output, exit_code) = run_with_default_context(
        include_bytes!("../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_data,
    );
    println!("Output: {:?}", output);
    assert_eq!(exit_code, 0);
    assert_eq!(output[0], 0xef);
}

#[test]
fn test_example_panic() {
    let input_data = include_bytes!("../../examples/panic/lib.wasm");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(
        from_utf8(&output).unwrap(),
        "panicked at examples/panic/lib.rs:17:9: it is panic time"
    );
    assert_eq!(exit_code, -71);
}

#[test]
fn test_example_router() {
    let input_data = include_bytes!("../../examples/router/lib.wasm");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    assert_eq!(&output, &hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    assert_eq!(exit_code, 0);
}

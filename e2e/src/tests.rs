use crate::helpers::{run_rwasm_with_evm_input, run_rwasm_with_raw_input};
use core::str::from_utf8;
use hex_literal::hex;

#[test]
fn test_example_greeting() {
    let output = run_rwasm_with_evm_input(
        include_bytes!("../../examples/greeting/lib.wasm").to_vec(),
        "Hello, World".as_bytes(),
    );
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.output.clone(), "Hello, World".as_bytes().to_vec());
}

#[test]
fn test_example_keccak256() {
    let output = run_rwasm_with_evm_input(
        include_bytes!("../../examples/hashing/lib.wasm").to_vec(),
        "Hello, World".as_bytes(),
    );
    assert_eq!(output.exit_code, 0);
    assert_eq!(
        output.output[0..32],
        hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529").to_vec()
    );
}

#[test]
fn test_example_rwasm() {
    let input_data = include_bytes!("../../examples/rwasm/lib.wasm");
    let output = run_rwasm_with_raw_input(
        include_bytes!("../../examples/rwasm/greeting.wasm").to_vec(),
        input_data,
        false,
    );
    assert_eq!(output.exit_code, 0);
}

#[test]
fn test_example_panic() {
    let input_data = include_bytes!("../../examples/panic/lib.wasm");
    let output = run_rwasm_with_raw_input(input_data.to_vec(), &[], false);
    assert_eq!(
        from_utf8(&output.output).unwrap(),
        "panicked at examples/panic/lib.rs:15:9: it is panic time"
    );
    assert_eq!(output.exit_code, -71);
}

#[test]
fn test_example_allocator() {
    let input_data = include_bytes!("../../examples/allocator/lib.wasm");
    let output = run_rwasm_with_raw_input(input_data.to_vec(), "Hello, World".as_bytes(), false);
    assert_eq!(&output.output, "Hello, World".as_bytes());
    assert_eq!(output.exit_code, 0);
}

#[test]
fn test_example_router() {
    let input_data = include_bytes!("../../examples/router/lib.wasm");
    let output = run_rwasm_with_raw_input(input_data.to_vec(), &hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"), false);
    assert_eq!(&output.output, &hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    assert_eq!(output.exit_code, 0);
}

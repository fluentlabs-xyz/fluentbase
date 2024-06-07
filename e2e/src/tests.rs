use crate::helpers::{run_rwasm_with_evm_input, run_rwasm_with_raw_input};
use fluentbase_poseidon::poseidon_hash;
use hex_literal::hex;

#[test]
fn test_greeting() {
    let output = run_rwasm_with_evm_input(
        include_bytes!("../../examples/greeting/lib.wasm").to_vec(),
        "Hello, World".as_bytes(),
    );
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.output.clone(), "Hello, World".as_bytes().to_vec());
}

#[test]
fn test_keccak256() {
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
fn test_rwasm() {
    let input_data = include_bytes!("../../examples/rwasm/lib.wasm");
    let output = run_rwasm_with_raw_input(
        include_bytes!("../../examples/rwasm/greeting.wasm").to_vec(),
        input_data,
        false,
    );
    assert_eq!(output.exit_code, 0);
}

use crate::{
    EXAMPLE_GREETING, EXAMPLE_PANIC, EXAMPLE_ROUTER_SOLIDITY, EXAMPLE_RWASM, EXAMPLE_TINY_KECCAK256,
};
use core::str::from_utf8;
use fluentbase_testing::run_with_default_context;
use hex_literal::hex;

#[test]
fn test_example_greeting() {
    let (output, exit_code) =
        run_with_default_context(EXAMPLE_GREETING.to_vec(), "Hello, World".as_bytes());
    assert_eq!(exit_code, 0);
    assert_eq!(output.clone(), "Hello, World".as_bytes().to_vec());
}

#[test]
fn test_example_keccak256() {
    let (output, exit_code) =
        run_with_default_context(EXAMPLE_TINY_KECCAK256.to_vec(), "Hello, World".as_bytes());
    assert_eq!(exit_code, 0);
    assert_eq!(
        output[0..32],
        hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529").to_vec()
    );
}

#[test]
fn test_example_rwasm() {
    let input_data = EXAMPLE_GREETING;
    let (output, exit_code) = run_with_default_context(EXAMPLE_RWASM.to_vec(), input_data);
    assert_eq!(exit_code, 0);
    assert_eq!(output[0], 0xef);
}

#[test]
fn test_example_panic() {
    let (output, exit_code) = run_with_default_context(EXAMPLE_PANIC.to_vec(), &[]);
    assert_eq!(from_utf8(&output[..]).unwrap(), "it's panic time",);
    assert_eq!(exit_code, -1);
}

#[test]
fn test_example_router() {
    let (output, exit_code) = run_with_default_context(EXAMPLE_ROUTER_SOLIDITY.to_vec(), &hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    assert_eq!(&output, &hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    assert_eq!(exit_code, 0);
}

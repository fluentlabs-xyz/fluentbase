use crate::utils::EvmTestingContext;
use core::str::from_utf8;
use fluentbase_genesis::EXAMPLE_GREETING_ADDRESS;
use fluentbase_sdk::{Address, Bytes};

#[test]
fn test_genesis_greeting() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        EXAMPLE_GREETING_ADDRESS,
        Bytes::default(),
        None,
        None,
    );

    assert!(result.is_success());
    println!("gas used (call): {}", result.gas_used());
    let bytes = result.output().unwrap_or_default();
    assert_eq!("Hello, World", from_utf8(bytes.as_ref()).unwrap());
}

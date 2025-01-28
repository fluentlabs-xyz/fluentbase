use crate::utils::EvmTestingContext;
use alloc::vec::Vec;
use fluentbase_sdk::{constructor::encode_constructor_params, Address, Bytes};
use hex_literal::hex;

#[test]
fn test_deploy_with_constructor_params() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let bytecode: Vec<u8> = include_bytes!("../../examples/constructor-params/lib.wasm").into();
    let constructor_params: Vec<u8> =
        hex!("68656c6c6fffffffffffffffffffffffffffffffffffffffffffffffffffffff").into();
    let encoded_params = encode_constructor_params(&constructor_params);
    let mut input: Vec<u8> = Vec::new();
    input.extend(bytecode);
    input.extend(encoded_params);
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, input.into());
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
        None,
        None,
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(constructor_params, bytes.to_vec());
}

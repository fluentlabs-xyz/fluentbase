use crate::utils::{EvmTestingContext};
use alloc::vec::Vec;
use fluentbase_sdk::{Address, Bytes};
use hex_literal::hex;

fn encode_leb128_simple(value: u32) -> Vec<u8> {
    assert!(value <= 127, "Value exceeds 1-byte LEB128 encoding limit");
    vec![value as u8]
}

fn encode_wasm_custom_section(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut section = Vec::new();
    section.push(0x00);
    let name_bytes = name.as_bytes();
    let name_length = encode_leb128_simple(name_bytes.len() as u32);
    let content_length = encode_leb128_simple((name_bytes.len() + payload.len() + name_length.len()) as u32);
    section.extend(content_length);
    section.extend(name_length);
    section.extend_from_slice(name_bytes);
    section.extend(payload);
    section
}

#[test]
fn test_deploy_with_constructor_params() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let bytecode: Vec<u8> = include_bytes!("../../examples/constructor-params/lib.wasm").into();
    let constructor_params: Vec<u8> = hex!("68656c6c6fffffffffffffffffffffffffffffffffffffffffffffffffffffff").into();
    let encoded_params = encode_wasm_custom_section("input", &constructor_params);
    let mut input: Vec<u8> = Vec::new();
    input.extend(bytecode);
    input.extend(encoded_params);
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        input.into(),
    );
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


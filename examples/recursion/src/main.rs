#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
use fluentbase_sdk::{
    derive::solidity_storage,
    entrypoint,
    Address,
    ContextReader,
    ExitCode,
    SharedAPI,
    U256,
};

solidity_storage! {
    mapping(Address => U256) Values;
}

pub fn deploy(_sdk: impl SharedAPI) {}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    assert_eq!(input_size, 8);
    let mut input_value = [0u8; 8];
    sdk.read(&mut input_value, 0);
    let mut input_value: u64 = u64::from_le_bytes(input_value);
    if input_value == 0 {
        return;
    } else {
        input_value -= 1;
        let input_value: [u8; 8] = input_value.to_le_bytes();
        let address = sdk.context().contract_address();
        let value = sdk.call(address, U256::default(), &input_value, None);
        assert_eq!(value.status, ExitCode::Ok);
    }
}

entrypoint!(main_entry, deploy);

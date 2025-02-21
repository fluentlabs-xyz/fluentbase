#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    derive::solidity_storage,
    func_entrypoint,
    Address,
    ContractContextReader,
    SharedAPI,
    U256,
};

solidity_storage! {
    mapping(Address => U256) Values;
}

pub fn main(mut sdk: impl SharedAPI) {
    let caller = sdk.context().contract_caller();
    let input_size = sdk.input_size();
    if input_size == 0 {
        let value = Values::get(&sdk, caller);
        sdk.write(&value.to_le_bytes::<32>());
    } else {
        let input = alloc_slice(input_size as usize);
        sdk.read(input, 0);
        let value = U256::from_le_slice(input);
        Values::set(&mut sdk, caller, value);
    }
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{testing::TestingContext, Address, Bytes, ContractContextV1, U256};
    use hex_literal::hex;

    #[test]
    fn test_simple_storage_set_and_get() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let test_value = U256::from(42);
        let sdk = TestingContext::default()
            .with_input(test_value.to_le_bytes::<32>())
            .with_contract_context(ContractContextV1 {
                caller: owner_address,
                ..Default::default()
            });
        main(sdk.clone());
        let sdk = sdk.with_input(Bytes::default());
        main(sdk.clone());
        let output = sdk.take_output();
        let retrieved_value = U256::from_le_slice(&output);
        assert_eq!(retrieved_value, test_value);
    }
}

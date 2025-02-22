#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{derive::solidity_storage, func_entrypoint, SharedAPI, U256};

solidity_storage! {
    U256 Value;
}

fn deploy(mut sdk: impl SharedAPI) {
    let mut input = [0u8; 32];
    sdk.read(&mut input, 0);
    let value = U256::from_le_bytes(input);
    sdk.write_storage(Value::SLOT, value);
}

fn main(mut sdk: impl SharedAPI) {
    let value = sdk.storage(&Value::SLOT);
    sdk.write(&value.to_le_bytes::<32>());
}

func_entrypoint!(main, deploy);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{address, testing::TestingContext, ContractContextV1, U256};

    #[test]
    fn test_constructor_params() {
        let context = ContractContextV1 {
            address: address!("1111111111111111111111111111111111111111"),
            bytecode_address: address!("2222222222222222222222222222222222222222"),
            caller: address!("3333333333333333333333333333333333333333"),
            is_static: false,
            value: U256::ZERO,
        };
        let sdk = TestingContext::default()
            .with_input(U256::from(123).to_le_bytes::<32>())
            .with_contract_context(context.clone());
        deploy(sdk.clone());
        let sdk = sdk.with_input(vec![]);
        main(sdk.clone());
        let output = sdk.take_output();
        let value = U256::from_le_slice(&output);
        assert_eq!(value, U256::from(123));
    }
}

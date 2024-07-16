#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    contracts::{EvmAPI, EvmSloadInput, EvmSstoreInput, PRECOMPILE_EVM},
    derive::solidity_storage,
    Address,
    U256,
};

solidity_storage! {
    U256[] Arr<EvmAPI>;
    mapping(Address => U256) Balance<EvmAPI>;
}

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::{
        codec::Encoder,
        contracts::EvmClient,
        runtime::TestingContext,
        Address,
        ContractInput,
        U256,
    };
    use fluentbase_types::address;
    use serial_test::serial;

    fn with_test_input<T: Into<Vec<u8>>>(input: T, caller: Option<Address>) -> TestingContext {
        let ctx = TestingContext::new().with_input(input);
        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = caller.unwrap_or_default();
        ctx.with_context(contract_input.encode_to_vec(0))
    }

    #[serial]
    #[test]
    pub fn test_client() {
        let ctx = with_test_input(
            vec![],
            Some(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
        )
        .with_devnet_genesis();
        let client = EvmClient::new(ctx.clone(), PRECOMPILE_EVM);
        let b: Balance<TestingContext, _> = Balance::new(&client);

        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); // 1000

        let slot = U256::from_str_radix("1", 10).unwrap();
        let input = EvmSstoreInput {
            index: slot,
            value: owner_balance,
        };

        b.client.sstore(input);

        let sload_input = EvmSloadInput { index: slot };

        let balance = b.client.sload(sload_input);

        assert_eq!(balance.value, owner_balance);
    }
    #[serial]
    #[test]
    pub fn test_arr() {
        let ctx = with_test_input(
            vec![],
            Some(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
        )
        .with_devnet_genesis();
        let client = EvmClient::new(ctx, PRECOMPILE_EVM);
        let arr: Arr<TestingContext, _> = Arr::new(&client);

        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); // 1000

        let index = U256::from_str_radix("0", 10).unwrap();

        arr.set(index, owner_balance);

        let output = arr.get(index);

        assert_eq!(output, owner_balance);
    }

    #[serial]
    #[test]
    pub fn test_storage() {
        let ctx = with_test_input(
            vec![],
            Some(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
        )
        .with_devnet_genesis();
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); // 1000

        let slot = U256::from_str_radix("1", 10).unwrap();
        let input = EvmSstoreInput {
            index: slot,
            value: owner_balance,
        };

        let client = EvmClient::new(ctx, PRECOMPILE_EVM);
        client.sstore(input);

        let sload_input = EvmSloadInput { index: slot };

        let balance = client.sload(sload_input);

        assert_eq!(balance.value, owner_balance);
    }
}

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use alloy_sol_types::SolValue;
use hex_literal::hex;

use fluentbase_sdk::{
    Address,
    codec::Encoder,
    contracts::{EvmAPI, EvmClient, EvmSloadInput, EvmSstoreInput, PRECOMPILE_EVM},
    derive::solidity_storage,
    LowLevelSDK,
    SharedAPI,
    U256,
};

solidity_storage! {
    U256[] Arr<EvmAPI>;
    mapping(Address => U256) Balance<EvmAPI>;
}

#[cfg(test)]
mod test {
    use alloy_sol_types::SolCall;
    use serial_test::serial;

    use fluentbase_sdk::{
        Address,
        Bytes,
        codec::Encoder,
        ContractInput,
        contracts::EvmClient,
        LowLevelSDK,
        U256,
    };

    use super::*;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) {
        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = caller.unwrap_or_default();

        LowLevelSDK::with_test_context(contract_input.encode_to_vec(0));
        let input: Bytes = input.into();
        LowLevelSDK::with_test_input(input.into());
    }

    fn get_output() -> Vec<u8> {
        LowLevelSDK::get_test_output()
    }
    #[serial]
    #[test]
    pub fn test_client() {
        let client = EvmClient::new(PRECOMPILE_EVM);
        let b = Balance::new(&client);

        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], Some(owner_address));
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
        let client = EvmClient::new(PRECOMPILE_EVM);
        let arr = Arr::new(&client);
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        LowLevelSDK::init_with_devnet_genesis();

        with_test_input(vec![], Some(owner_address));
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); // 1000

        let index = U256::from_str_radix("0", 10).unwrap();

        arr.set(index, owner_balance);

        let output = arr.get(index);

        assert_eq!(output, owner_balance);
    }

    #[serial]
    #[test]
    pub fn test_storage() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], Some(owner_address));
        let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); // 1000

        let slot = U256::from_str_radix("1", 10).unwrap();
        let input = EvmSstoreInput {
            index: slot,
            value: owner_balance,
        };

        let client = EvmClient::new(PRECOMPILE_EVM);
        client.sstore(input);

        let sload_input = EvmSloadInput { index: slot };

        let balance = client.sload(sload_input);

        assert_eq!(balance.value, owner_balance);
    }
}

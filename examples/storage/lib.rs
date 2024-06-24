#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloy_sol_types::{sol, SolValue};
use fluentbase_sdk::{
    contracts::{
        EvmAPI, EvmSloadInput, EvmSloadOutput, EvmSstoreInput, EvmSstoreOutput, PRECOMPILE_EVM,
    },
    derive::{client, signature, solidity_storage},
    Address, LowLevelSDK, SharedAPI, U256,
};
use hex_literal::hex;

// #[client(mode = "codec")]
// pub trait EvmStorageAPI {
//     #[signature("sload(u256)")]
//     fn sload(&self, input: EvmSloadInput) -> EvmSloadOutput;

//     #[signature("sstore(u256,u256)")]
//     fn sstore(&self, input: EvmSstoreInput) -> EvmSstoreOutput;
// }

// solidity_storage! {
//     mapping(Address => U256) BalancesStorage<EvmStorageAPI>;
//     mapping(Address => mapping(Address => U256)) AllowanceStorage<EvmStorageAPI>;
// }

struct EvmStorageClient {
    pub address: fluentbase_sdk::Address,
    pub fuel: u32,
}
impl EvmStorageClient {
    pub fn new(address: fluentbase_sdk::Address) -> EvmStorageClient {
        Self {
            address,
            fuel: u32::MAX,
        }
    }
}
impl EvmStorageClient {
    fn sload(&self, input: EvmSloadInput) -> EvmSloadOutput {
        use fluentbase_sdk::codec::Encoder;
        use fluentbase_sdk::types::CoreInput;
        let core_input = CoreInput {
            method_id: 308662670u32,
            method_data: input,
        }
        .encode_to_vec(0);
        let (output, exit_code) =
            fluentbase_sdk::contracts::call_system_contract(&self.address, &core_input, self.fuel);
        if exit_code != 0 {
            {
                panic!("system contract call failed with exit code: {}", exit_code,);
            };
        }
        let mut decoder = fluentbase_sdk::codec::BufferDecoder::new(&output);
        let mut result = EvmSloadOutput::default();
        EvmSloadOutput::decode_body(&mut decoder, 0, &mut result);
        result
    }
    fn sstore(&self, input: EvmSstoreInput) -> EvmSstoreOutput {
        use fluentbase_sdk::codec::Encoder;
        use fluentbase_sdk::types::CoreInput;
        let core_input = CoreInput {
            method_id: 3249940432u32,
            method_data: input,
        }
        .encode_to_vec(0);
        let (output, exit_code) =
            fluentbase_sdk::contracts::call_system_contract(&self.address, &core_input, self.fuel);
        if exit_code != 0 {
            {
                panic!("system contract call failed with exit code: {}", exit_code,);
            };
        }
        let mut decoder = fluentbase_sdk::codec::BufferDecoder::new(&output);
        let mut result = EvmSstoreOutput::default();
        EvmSstoreOutput::decode_body(&mut decoder, 0, &mut result);
        result
    }
}
#[cfg(test)]
mod test {
    use super::*;
    use alloc::boxed::Box;
    use alloy_sol_types::SolCall;
    use fluentbase_sdk::{codec::Encoder, Address, Bytes, ContractInput, LowLevelSDK, U256};
    use serial_test::serial;

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

    // TODO: current
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

        let client = EvmStorageClient::new(PRECOMPILE_EVM);
        client.sstore(input);

        let sload_input = EvmSloadInput { index: slot };

        let balance = client.sload(sload_input);

        assert_eq!(balance.value, owner_balance);
    }
}

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    basic_entrypoint,
    bytes::Buf,
    derive::{function_id, router, Contract},
    Bytes,
    ContractContextReader,
    SharedAPI,
};

#[derive(Contract)]
struct Multicall<SDK> {
    sdk: SDK,
}

pub trait MulticallAPI {
    fn multicall(&mut self, data: Vec<Bytes>) -> Vec<Bytes>;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> MulticallAPI for Multicall<SDK> {
    #[function_id("multicall(bytes[])")]
    fn multicall(&mut self, data: Vec<Bytes>) -> Vec<Bytes> {
        // Use target address from context - this is the address of original contract
        let target_addr = self.sdk.context().contract_address();
        let mut results = Vec::with_capacity(data.len());

        for call_data in data {
            let chunk = call_data.chunk();
            // Always delegate call to the target contract address
            let (output, exit_code) = self.sdk.delegate_call(target_addr, chunk, 0);

            if exit_code != 0 {
                panic!("Multicall: delegate call failed");
            }

            results.push(output);
        }

        results
    }
}

impl<SDK: SharedAPI> Multicall<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(Multicall);
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_sdk::{
        codec::SolidityABI,
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContext,
    };

    fn prepare_testing_context<T: Into<Vec<u8>>>(
        input: T,
        contract_addr: Address,
    ) -> (TestingContext, JournalState<TestingContext>) {
        let native_sdk = TestingContext::empty().with_input(input);
        let sdk = JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: Address::default(),
                address: contract_addr,
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(native_sdk.clone());
        (native_sdk, sdk)
    }

    fn verify_sol_encoding<T: SolCall>(call: &T, input: &[u8]) {
        assert_eq!(
            hex::encode(&input[4..]),
            hex::encode(&call.abi_encode()[4..])
        );
    }

    #[test]
    fn test_multicall_input_encoding_roundtrip() {
        let greeting = hex::decode("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000")
        .unwrap();
        let custom_greeting =
        hex::decode("36b83a9a00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000015437573746f6d2048656c6c6f2c20576f726c6421210000000000000000000000")
        .unwrap();

        let inputs = vec![Bytes::from(greeting), Bytes::from(custom_greeting)];

        let multicall_input = MulticallCall::new((inputs.clone(),)).encode();

        sol!(function multicall(bytes[] data););
        let multicall_sol = multicallCall {
            data: inputs.clone(),
        };

        verify_sol_encoding(&multicall_sol, &multicall_input);
        println!(
            "multicall(bytes[]) input: {:?}",
            hex::encode(&multicall_input)
        );

        let decoded_inputs: Vec<Bytes> = SolidityABI::decode(&&multicall_input[4..], 0)
            .expect("failed to decode multicall inputs");

        assert_eq!(inputs, decoded_inputs);
    }
}

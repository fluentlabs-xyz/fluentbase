#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use client_solidity_api::{RouterAPI, ROUTER};
use fluentbase_codec::{Encoder, SolidityABI};
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{client, router, Contract},
    Address,
    SharedAPI,
    U256,
};

basic_entrypoint!(ROUTER);

#[cfg(test)]
mod tests {
    use super::*;
    use client_solidity_api::{greeting_input, RouterAPIClient};
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::{RuntimeContextWrapper, TestingContext},
        shared::SharedContextImpl,
        Address,
        ContractContext,
        NativeAPI,
    };

    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }

    // native  - std (bindings), nostd (runtimecontext wrapper)
    // shared sdk - wrapper over native sdk (to determine ee: 1. isolated journal (Journal State))
    fn create_sdk() -> JournalState<fluentbase_sdk::runtime::RuntimeContextWrapper> {
        // native sdk - low level api to interact with the contract
        let native_sdk = TestingContext::empty();

        // Journal State - api to interact with blockchain state (storage, events, etc)
        JournalState::empty(native_sdk.clone())
    }

    #[test]
    fn test_input() {
        let msg = String::from("Hello World");
        let input = greeting_input(msg);

        println!("{:?}", hex::encode(input));
    }

    #[test]
    fn test_contract_works() {
        let sdk: JournalState<RuntimeContextWrapper> = create_sdk();
        let contract: ROUTER<JournalState<RuntimeContextWrapper>> = ROUTER::new(sdk);

        contract.deploy();

        // get address of the contract
        let contract_address = contract.sdk.contract_context().address;

        let mut client = RouterAPIClient::new(contract.sdk, contract_address);

        let msg = "Hello, World".to_string();
        let value = U256::from(0);
        let gas_limit = 10_000_000;
        let input = greeting_input(msg.clone());

        let result = client.greeting(input, value, gas_limit);

        // assert_eq!(result, msg);
    }
}

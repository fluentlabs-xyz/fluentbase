#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::{String, ToString};
use core::{str::FromStr, u64};
use fluentbase_codec::{Encoder, SolidityABI};
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{client, router, Contract},
    Address,
    SharedAPI,
    U256,
};

trait RouterAPI {
    fn greeting(&mut self, message: String) -> String;
    // fn custom_greeting(&mut self, message: String) -> String;
}

pub struct RouterAPIClient<SDK> {
    pub sdk: SDK,
}

impl<SDK: SharedAPI> RouterAPIClient<SDK> {
    pub fn new(sdk: SDK) -> Self {
        Self { sdk }
    }
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPIClient<SDK> {
    #[function_id("greeting(string)", validate(false))]
    pub fn greeting(
        &mut self,
        contract_address: Address,
        value: U256,
        gas_limit: u64,
        message: String,
    ) -> String {
        // create input for target contract
        let input = target_contract_input(message.clone());

        // check if the caller has enough funds to make the call
        let tx_context = self.sdk.tx_context();
        if tx_context.value < value {
            panic!("insufficient funds");
        }
        if tx_context.gas_limit < gas_limit {
            panic!("insufficient gas");
        }

        // call the target contract
        let (output, exit_code) = self.sdk.call(contract_address, value, &input, gas_limit);

        // check if the call was successful
        if exit_code != 0 {
            panic!("call failed with exit code: {}", exit_code)
        }

        // decode the result
        let mut result_buf = fluentbase_codec::bytes::BytesMut::new();
        if fluentbase_codec::encoder::SolidityABI::<(String,)>::is_dynamic() {
            result_buf.extend(::fluentbase_sdk::U256::from(32).to_be_bytes::<32>());
        }
        result_buf.extend(output);

        let result =
            fluentbase_codec::encoder::SolidityABI::<(String,)>::decode(&result_buf.freeze(), 0)
                .expect("failed to decode result");

        result.0 + " from RouterAPIClient"
    }

    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

pub fn target_contract_input(message: String) -> alloc::vec::Vec<u8> {
    let mut input = alloc::vec![0u8; 4];
    input.copy_from_slice(&[248u8, 25u8, 78u8, 72u8]); // keccak256("greeting(string)")[:4]

    let mut buf = fluentbase_codec::bytes::BytesMut::new();
    fluentbase_codec::encoder::SolidityABI::encode(&(message,), &mut buf, 0).unwrap();
    let encoded_args = buf.freeze();

    let clean_args = if fluentbase_codec::encoder::SolidityABI::<(String,)>::is_dynamic() {
        encoded_args[32..].to_vec()
    } else {
        encoded_args.to_vec()
    };
    input.extend(clean_args);

    input
}
pub fn client_input(
    contract_address: Address,
    value: U256,
    gas_limit: u64,
    message: String,
) -> alloc::vec::Vec<u8> {
    let mut input = alloc::vec![0u8; 4];
    input.copy_from_slice(&[248u8, 25u8, 78u8, 72u8]); // keccak256("greeting(string)")[:4]

    let mut buf = fluentbase_codec::bytes::BytesMut::new();
    fluentbase_codec::encoder::SolidityABI::encode(
        &(contract_address, value, gas_limit, message),
        &mut buf,
        0,
    )
    .unwrap();
    let encoded_args = buf.freeze();

    let clean_args =
        if fluentbase_codec::encoder::SolidityABI::<(Address, U256, u64, String)>::is_dynamic() {
            encoded_args[32..].to_vec()
        } else {
            encoded_args.to_vec()
        };
    input.extend(clean_args);

    input
}

basic_entrypoint!(RouterAPIClient);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        address,
        journal::{JournalState, JournalStateBuilder},
        runtime::{RuntimeContextWrapper, TestingContext},
        shared::SharedContextImpl,
        Address,
        ContractContext,
        NativeAPI,
    };
    use hex::FromHex;
    use hex_literal::hex;

    //     fn with_test_input<T: Into<Vec<u8>>>(
    //         input: T,
    //         caller: Option<Address>,
    //     ) -> JournalState<TestingContext> {
    //         JournalStateBuilder::default()
    //             .with_contract_context(ContractContext {
    //                 caller: caller.unwrap_or_default(),
    //                 ..Default::default()
    //             })
    //             .with_devnet_genesis()
    //             .build(TestingContext::empty().with_input(input))
    //     }

    //     // native  - std (bindings), nostd (runtimecontext wrapper)
    //     // shared sdk - wrapper over native sdk (to determine ee: 1. isolated journal (Journal
    // State))     fn create_sdk() -> JournalState<fluentbase_sdk::runtime::RuntimeContextWrapper> {
    //         // native sdk - low level api to interact with the contract
    //         let native_sdk = TestingContext::empty();

    //         // Journal State - api to interact with blockchain state (storage, events, etc)
    //         JournalState::empty(native_sdk.clone())
    //     }

    #[test]
    fn test_input() {
        let msg = String::from("Hello World");
        let contract_address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
        let input = client_input(contract_address, U256::from(0), 21_000, msg);

        println!("{:?}", hex::encode(input));
    }

    //     #[test]
    //     fn test_contract_works() {
    //         let sdk: JournalState<RuntimeContextWrapper> = create_sdk();
    //         let contract: ROUTER<JournalState<RuntimeContextWrapper>> = ROUTER::new(sdk);

    //         contract.deploy();

    //         // get address of the contract
    //         let contract_address = contract.sdk.contract_context().address;

    //         let mut client = RouterAPIClient::new(contract.sdk, contract_address);

    //         let msg = "Hello, World".to_string();
    //         let value = U256::from(0);
    //         let gas_limit = 10_000_000;
    //         let input = greeting_input(msg.clone());

    //         let result = client.greeting(input, value, gas_limit);

    //         // assert_eq!(result, msg);
    //     }
}

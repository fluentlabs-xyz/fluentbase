#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_codec::{Encoder, SolidityABI};
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{client, router, Contract},
    Address,
    Bytes,
    SharedAPI,
    U256,
};

pub trait RouterAPI {
    fn greeting(&mut self, message: String) -> String;
    fn custom_greeting(&mut self, message: String) -> String;
    fn fallback(&mut self);
}

pub struct RouterAPIClient<SDK> {
    pub sdk: SDK,
    pub contract_address: Address,
}

impl<SDK: SharedAPI> RouterAPIClient<SDK> {
    pub fn new(sdk: SDK, contract_address: Address) -> Self {
        Self {
            sdk,
            contract_address,
        }
    }

    pub fn greeting(&mut self, input: alloc::vec::Vec<u8>, value: U256, gas_limit: u64) -> Bytes {
        let (result, exit_code) = self
            .sdk
            .call(self.contract_address, value, &input, gas_limit);

        if exit_code != 0 {
            panic!("call failed with exit code: {}", exit_code)
        }
        result
        // let mut result_buf = fluentbase_codec::bytes::BytesMut::new();
        // if fluentbase_codec::encoder::SolidityABI::<(String,)>::is_dynamic() {
        //     result_buf.extend(::fluentbase_sdk::U256::from(32).to_be_bytes::<32>());
        // }
        // result_buf.extend(result);

        // fluentbase_codec::encoder::SolidityABI::<String>::decode(&result_buf.freeze(), 0)
        //     .expect("failed to decode result")
    }
}

pub fn greeting_input(message: String) -> alloc::vec::Vec<u8> {
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

#[derive(Contract)]
pub struct ROUTER<SDK> {
    pub sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&mut self, message: String) -> String {
        panic!("greeting is not implemented yet ;)");
        self.sdk.write(message.clone().as_bytes());
        message
    }

    #[function_id("customGreeting(string)")]
    fn custom_greeting(&mut self, message: String) -> String {
        message
    }

    fn fallback(&mut self) {
        panic!("fallback is not implemented yet ;)");
    }
}

impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

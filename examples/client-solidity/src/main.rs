#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use core::u64;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{client, router},
    Address,
    SharedAPI,
    U256,
};

/// RouterAPIClient
#[client(mode = "solidity")]
trait RouterAPI {
    #[function_id("greeting(string)", validate(false))]
    fn greeting(&mut self, message: String) -> String;

    #[function_id("customGreeting(string)", validate(false))]
    fn custom_greeting(&self, message: String) -> String;
}

/// Create a contract for test purpose
#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPIClient<SDK> {
    pub fn greeting_client(
        &mut self,
        contract_address: Address,
        value: U256,
        gas_limit: u64,
        message: String,
    ) -> String {
        self.greeting(contract_address, value, gas_limit, message)
    }

    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(RouterAPIClient);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{address, bytes::BytesMut, codec::SolidityABI, Address};

    #[test]
    fn generate_target_contract_input() {
        let msg = "Hello World".to_string();
        let input = GreetingCall::new((msg.clone(),)).encode();
        println!("{:?}", hex::encode(input));
    }

    #[test]
    fn test_client_contract_input_encoding() {
        let msg = "Hello World".to_string();
        let contract_address = address!("f91c20c0cafbfdc150adff51bbfc5808edde7cb5");
        let value = U256::from(0);
        let gas_limit = 21_000;
        let input =
            GreetingClientCall::new((contract_address, value, gas_limit, msg.clone())).encode();
        let expected_encoded = "f60ea708000000000000000000000000f91c20c0cafbfdc150adff51bbfc5808edde7cb5000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000052080000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000b48656c6c6f20576f726c64000000000000000000000000000000000000000000";
        assert_eq!(hex::encode(&input), expected_encoded);
        let mut decode_buf = BytesMut::new();
        decode_buf.extend_from_slice(&U256::from(32).to_be_bytes::<32>());
        decode_buf.extend_from_slice(&input[4..]);
        let decoded_input: (Address, U256, u64, String) =
            SolidityABI::decode(&decode_buf.freeze(), 0).unwrap();
        assert_eq!(decoded_input, (contract_address, value, gas_limit, msg));
    }
}

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{client, router},
    Address,
    SharedAPI,
    U256,
};

/// RouterAPIClient
#[client(mode = "fluent")]
trait RouterAPI {
    #[function_id("greeting(string)", validate(false))]
    fn greeting(&mut self, message: String) -> String;
}

/// Create contract for test purpose

#[router(mode = "fluent")]
impl<SDK: SharedAPI> RouterAPIClient<SDK> {
    pub fn greeting_client(
        &mut self,
        contract_address: Address,
        value: U256,
        gas_limit: u64,
        message: String,
    ) -> String {
        self.greeting(contract_address, value, gas_limit, message).0
    }

    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(RouterAPIClient);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{address, bytes::BytesMut, codec::CompactABI, Address};

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

        let expected_encoded = "f60ea708f91c20c0cafbfdc150adff51bbfc5808edde7cb500000000000000000000000000000000000000000000000000000000000000000852000000000000440000000b00000048656c6c6f20576f726c6400"
        ;

        assert_eq!(hex::encode(&input), expected_encoded);

        let mut decode_buf = BytesMut::new();

        decode_buf.extend_from_slice(&(4 as u32).to_le_bytes());

        decode_buf.extend_from_slice(&input[4..]);

        let decoded_input: (Address, U256, u64, String) =
            CompactABI::decode(&decode_buf.freeze(), 0).unwrap();

        assert_eq!(decoded_input, (contract_address, value, gas_limit, msg));
    }
}

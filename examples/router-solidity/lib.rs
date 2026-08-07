#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    FixedBytes, SharedAPI,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
    fn byte_array(&self, data: [u8; 32]) -> [u8; 32];
    fn fixed_bytes(&self, data: FixedBytes<32>) -> FixedBytes<32>;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[function_id("customGreeting(string)")]
    fn custom_greeting(&self, message: String) -> String {
        message
    }

    // `[u8; 32]` is encoded one word per element, so it must advertise `uint8[32]`, not `bytes32`
    #[function_id("byteArray(uint8[32])", validate(true))]
    fn byte_array(&self, data: [u8; 32]) -> [u8; 32] {
        data
    }

    // `bytes32` is a single right-padded word, and `FixedBytes<32>` is the type that encodes it
    #[function_id("fixedBytes(bytes32)", validate(true))]
    fn fixed_bytes(&self, data: FixedBytes<32>) -> FixedBytes<32> {
        data
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(App);

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_testing::TestingContextImpl;

    #[test]
    fn test_greeting() {
        let input = GreetingCall::new(("Hello, World".to_string(),)).encode();
        sol!(
            function greeting(string message);
        );
        let input_sol = greetingCall {
            message: "Hello, World".to_string(),
        }
        .abi_encode();
        assert_eq!(hex::encode(&input), hex::encode(&input_sol));
        println!("greeting(string) input: {:?}", hex::encode(&input));
        let sdk = TestingContextImpl::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(encoded_output));
        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, "Hello, World".to_string());
    }

    #[test]
    fn test_custom_greeting() {
        let s = String::from("Custom Hello, World!!");
        let input = CustomGreetingCall::new((s.clone(),)).encode();
        // SOL INPUT
        sol!(
            function customGreeting(string message);
        );
        let input_sol = customGreetingCall { message: s.clone() }.abi_encode();
        assert_eq!(hex::encode(&input), hex::encode(&input_sol));
        println!("customGreeting(string) input: {:?}", hex::encode(&input));
        let sdk = TestingContextImpl::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(encoded_output));
        let output = CustomGreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, s);
    }

    /// A `[u8; 32]` parameter advertises `uint8[32]`, and the generated calldata has to be byte
    /// for byte what Solidity produces for `uint8[32]` — selector *and* body. When the selector
    /// said `bytes32` instead, canonical calldata reached this route and then failed to decode.
    #[test]
    fn test_byte_array_matches_solidity() {
        let data: [u8; 32] = core::array::from_fn(|i| (i + 1) as u8);
        let input = ByteArrayCall::new((data,)).encode();
        sol!(
            function byteArray(uint8[32] data);
        );
        let input_sol = byteArrayCall { data }.abi_encode();
        assert_eq!(hex::encode(&input), hex::encode(&input_sol));

        let sdk = TestingContextImpl::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        let output = ByteArrayReturn::decode(&encoded_output.as_slice()).unwrap();
        assert_eq!(output.0 .0, data);
    }

    /// The `bytes32` counterpart: `FixedBytes<32>` is the type that carries a real `bytesN` codec,
    /// so it round-trips against a Solidity `bytes32` encoder.
    #[test]
    fn test_fixed_bytes_matches_solidity() {
        let data = FixedBytes::<32>::from([7u8; 32]);
        let input = FixedBytesCall::new((data,)).encode();
        sol!(
            function fixedBytes(bytes32 data);
        );
        let input_sol = fixedBytesCall { data }.abi_encode();
        assert_eq!(hex::encode(&input), hex::encode(&input_sol));

        let sdk = TestingContextImpl::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        let output = FixedBytesReturn::decode(&encoded_output.as_slice()).unwrap();
        assert_eq!(output.0 .0, data);
    }
}

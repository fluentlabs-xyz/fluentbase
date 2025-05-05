#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract},
    SharedAPI,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
}

// TODO(d1r1): Prevent creation of intermediate directories during attribute typing.
// When IDE runs automatic cargo check while user is typing the artifacts path,
// unwanted directories are created (a/, ar/, art/, etc). Consider using a deferred
// approach where artifacts location is stored in configuration rather than directly
// in attribute parameters.
#[router(
    mode = "solidity",
    artifacts = "/Users/danilnaumov/dev/src/github.com/fluentlabs-xyz/fluentbase-v3/examples/router-solidity/artifacts"
)]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[function_id("customGreeting(string)")]
    fn custom_greeting(&self, message: String) -> String {
        message
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
    use fluentbase_sdk::testing::TestingContext;

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
        let sdk = TestingContext::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(&encoded_output));
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
        let sdk = TestingContext::default().with_input(input);
        let mut router = App::new(sdk.clone());
        router.deploy();
        router.main();
        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(&encoded_output));
        let output = CustomGreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, s);
    }
}

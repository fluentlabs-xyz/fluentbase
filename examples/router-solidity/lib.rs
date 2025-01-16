#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract},
    SharedAPI,
};

#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[function_id("customGreeting(string)")]
    fn custom_greeting(&self, message: String) -> String {
        message
    }
}

impl<SDK: SharedAPI> ROUTER<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(ROUTER);

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_greeting() {
        let s = String::from("Hello, World!!");

        let greeting_call = GreetingCall::new((s.clone(),));

        let input = greeting_call.encode();

        // SOL INPUT
        sol!(
            function greeting(string message);
        );

        let input_sol = greetingCall { message: s.clone() }.abi_encode();

        assert_eq!(hex::encode(&input), hex::encode(&input_sol));

        println!("greeting(string) input: {:?}", hex::encode(&input));
        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(&encoded_output));
        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, s);
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
        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(&encoded_output));
        let output = CustomGreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, s);
    }
}

// // greeting call (Hello, World!)
// f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000d48656c6c6f2c20576f726c642100000000000000000000000000000000000000

// // custom greeting call (Custom Hello, World!)
// 36b83a9a00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000014437573746f6d2048656c6c6f2c20576f726c6421000000000000000000000000

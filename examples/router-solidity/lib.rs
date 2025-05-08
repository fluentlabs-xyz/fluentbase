#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
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

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    #[function_id("greeting(string)", validate(true))]
    fn greeting(&self, message: String) -> String {
        message
    }

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

    #[test]
    fn test_custom_greeting() {
        let s = String::from("Custom Hello, World!!");
        let input = CustomGreetingCall::new((s.clone(),)).encode();
        let expected_output = "36b83a9a00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000015437573746f6d2048656c6c6f2c20576f726c6421210000000000000000000000";

        assert_eq!(hex::encode(&input), expected_output);
    }
}

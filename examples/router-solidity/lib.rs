#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    Address,
    SharedAPI,
    U256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, addr: Address, amount: U256, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    fn greeting(&self, addr: Address, amount: U256, message: String) -> String {
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
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_greeting() {
        let input = router_api_app::GreetingCall::new((
            Address::from_slice(&hex::decode("2fbafa312f219b32c007f683491aaf54d9173561").unwrap()),
            U256::from(12345),
            "Hello, World".to_string(),
        ))
        .encode();

        let expected_output = "4e8563f80000000000000000000000002fbafa312f219b32c007f683491aaf54d917356100000000000000000000000000000000000000000000000000000000000030390000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000c48656c6c6f2c20576f726c640000000000000000000000000000000000000000";

        assert_eq!(hex::encode(&input), expected_output);
    }

    #[test]
    fn test_custom_greeting() {
        let s = String::from("Custom Hello, World!!");
        let input = router_api_app::CustomGreetingCall::new((s.clone(),)).encode();
        let expected_output = "36b83a9a00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000015437573746f6d2048656c6c6f2c20576f726c6421210000000000000000000000";

        assert_eq!(hex::encode(&input), expected_output);
    }
}

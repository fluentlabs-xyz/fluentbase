#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature, Contract},
    NativeAPI,
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
    #[signature("function greeting(string message) external returns (string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[signature("customGreeting(string)")]
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
    use alloy_sol_types::SolCall;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};
    use hex_literal::hex;

    #[test]
    fn test_contract_method_works() {
        // form test input
        let input = hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000");
        let msg = greetingCall::abi_decode(&input, true).unwrap_or_else(|e| {
            panic!("Failed to decode input {:?} {:?}", "msg", e,);
        });
        let sdk = TestingContext::new().with_input(input);
        // run router
        let mut greeting = ROUTER::new(JournalState::empty(sdk.clone()));
        greeting.deploy();
        greeting.main();
        // check result
        let test_output = sdk.take_output();
        assert_eq!(test_output,
    hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"
    ));
    }
}

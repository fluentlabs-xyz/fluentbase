#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature},
    SharedAPI,
};

#[derive(Default)]
struct ROUTER;

pub trait RouterAPI {
    fn deploy<SDK: SharedAPI>(&self);
    fn greeting(&self, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl RouterAPI for ROUTER {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }

    #[signature("function greeting(string message) external returns (string)")]
    fn greeting(&self, message: String) -> String {
        message
    }

    #[signature("customGreeting(string)")]
    fn custom_greeting(&self, message: String) -> String {
        message
    }
}

basic_entrypoint!(ROUTER);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;
    use hex_literal::hex;

    #[test]
    fn test_contract_method_works() {
        // form test input
        let input = hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000");
        let msg = greetingCall::abi_decode(&input, true).unwrap_or_else(|e| {
            panic!("Failed to decode input {:?} {:?}", "msg", e,);
        });
        LowLevelSDK::with_test_input(input.into());
        // run router
        let greeting = ROUTER::default();
        greeting.deploy::<LowLevelSDK>();
        greeting.main::<LowLevelSDK>();
        // check result
        let test_output = LowLevelSDK::get_test_output();
        assert_eq!(test_output, hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"));
    }
}

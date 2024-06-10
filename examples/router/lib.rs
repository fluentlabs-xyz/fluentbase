#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{basic_entrypoint, router, signature, SharedAPI};

#[derive(Default)]
struct ROUTER;

pub trait RouterAPI {
    fn greeting(&self) -> String;
}

#[router(mode = "solidity")]
impl RouterAPI for ROUTER {
    #[signature("function greeting(string message) external returns (string)")]
    pub fn greeting(&self, message: String) -> String {
        message
    }
}

impl ROUTER {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(ROUTER);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_method_works() {
        let input = "f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c48656c6c6f20776f726c64210000000000000000000000000000000000000000";
        let input = hex::decode(input).expect("Failed to decode hex string");

        let msg = match greetingCall::abi_decode(&input, true) {
            Ok(decoded) => decoded.message,
            Err(e) => {
                {
                    panic!("Failed to decode input {:?} {:?}", "msg", e,);
                };
            }
        };

        LowLevelSDK::with_test_input(input.clone());

        let greeting = ROUTER::default();
        greeting.deploy::<LowLevelSDK>();
        greeting.main::<LowLevelSDK>();

        let test_output = LowLevelSDK::get_test_output();
        let res = greetingCall::abi_decode_returns(&test_output, false).unwrap();

        assert_eq!(msg, res._0);
    }
}

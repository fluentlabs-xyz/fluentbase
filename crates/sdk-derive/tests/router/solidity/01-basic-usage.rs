#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
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
// we need to specify main fn to avoid
// error[E0601]: main function not found in crate $CRATE
fn main() {}
#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{sol, SolCall};
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext, Address, Bytes};

    #[test]
    fn test_contract_works() {
        let s = String::from("Hello, World!!");

        let greeting_call = GreetingCall::new((s.clone(),));

        let input = greeting_call.encode();

        // SOL INPUT
        sol!(
            function buying(string message);
        );

        let buying_call_sol = buyingCall { message: s.clone() };

        let byuing_call_input_sol = buying_call_sol.abi_encode();

        assert_eq!(
            hex::encode(&input[4..]),
            hex::encode(&byuing_call_input_sol[4..])
        );

        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        assert_eq!(output.0 .0, s);
    }
}
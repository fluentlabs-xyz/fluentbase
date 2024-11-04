#![cfg_attr(target_arch = "wasm32", no_std)]
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
    // fn custom_greeting(&self, message: Bytes) -> Bytes;
}

#[router(mode = "fluent")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(string)")] // 0xf8194e48
    fn greeting(&self, message: String) -> String {
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
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let m = "Hello World".to_string();

        let greeting_call = GreetingCall::new((m.clone(),));

        let input = greeting_call.encode();

        println!("Input: {:?}", hex::encode(&input));
        println!("call contract...");
        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        println!("encoded output: {:?}", &encoded_output);

        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output);
        assert_eq!(output.0 .0, m);
    }
}

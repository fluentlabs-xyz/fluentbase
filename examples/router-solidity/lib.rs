#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    Address,
    Bytes,
    SharedAPI,
};

#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: Bytes, caller: Address) -> Bytes;
    // fn custom_greeting(&self, message: Bytes) -> Bytes;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(bytes,address)")] // 0xf8194e48
    fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
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
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext, Bytes};

    #[test]
    fn test_contract_works() {
        let b = Bytes::from("Hello, World!!".as_bytes());
        let a = Address::repeat_byte(0xAA);

        let greeting_call = GreetingCall::new((b.clone(), a.clone()));

        let input = greeting_call.encode();

        // SOL INPUT
        sol!(
            function buying(bytes message, address caller);
        );

        let buying_call_sol = buyingCall {
            message: b.clone(),
            caller: a.clone(),
        };

        let byuing_call_input_sol = buying_call_sol.abi_encode();

        assert_eq!(
            hex::encode(&input[4..]),
            hex::encode(&byuing_call_input_sol[4..])
        );

        println!("Input: {:?}", hex::encode(&input));
        println!("call contract...");
        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        println!("output: {:?}", hex::encode(&encoded_output));
        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        println!("output: {:?}", &output.0);
        assert_eq!(output.0 .0, b);
    }
}

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Codec,
    derive::{function_id, router, Contract, SolidityABI},
    SharedAPI,
    U256,
};

#[derive(Codec, Debug, Clone, SolidityABI)]
pub struct Point {
    x: u64,
    y: U256,
}

#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: String) -> String;
    fn custom_greeting(&self, message: String) -> String;
    fn complex(&self, point: Point) -> Point;
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

    fn complex(&self, point: Point) -> Point {
        point
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
        assert_eq!(output.0 .0, s);
    }

    #[test]
    fn test_complex_works() {
        let point = Point {
            x: 42,
            y: U256::from(100),
        };

        let complex_call = ComplexCall::new((point.clone(),));
        let input = complex_call.encode();

        sol!(
            function complex((uint64,uint256) point);
        );

        let complex_call_sol = complexCall {
            point: (point.x, point.y.into()),
        };
        let complex_call_input_sol = complex_call_sol.abi_encode();

        assert_eq!(
            hex::encode(&input[4..]),
            hex::encode(&complex_call_input_sol[4..])
        );

        println!("Input: {:?}", hex::encode(&input));

        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();
        println!("Output: {:?}", hex::encode(&encoded_output));
        let output = ComplexReturn::decode(&encoded_output.as_slice()).unwrap();

        assert_eq!(output.0 .0.x, point.x);
        assert_eq!(output.0 .0.y, point.y);
    }
}

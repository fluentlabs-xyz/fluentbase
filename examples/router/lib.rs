#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use alloy_sol_types::SolCall;
use bytes::{Buf, BytesMut};
use codec2::{encoder::SolidityABI, error::CodecError};
use core::ops::Deref;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature, Contract},
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
    #[signature("function greeting(bytes)")] // 0xf8194e48
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
    // TODO: we need to import it inside codec2 to avoid aditional import
    use alloc::fmt;
    use alloy_sol_types::sol;
    use bytes::BufMut;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext, Bytes};
    use hex_literal::hex;
    use std::io::Read;

    #[test]
    fn test_encoding_decoding() {
        let original = Bytes::from("Hello, World!!".as_bytes());
        // let router = Bytes::
        let mut buf = BytesMut::new();

        SolidityABI::encode(&original, &mut buf, 0).unwrap();

        let encoded = buf.freeze();
        let expected_encoded = "0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000";
        println!("{:?}", hex::encode(&encoded));

        assert_eq!(hex::encode(&encoded), expected_encoded);

        println!("Decoding...");
        let decoded = SolidityABI::<Bytes>::decode(&encoded, 0).unwrap();
        println!("{:?}", decoded);
        assert_eq!(decoded, original);
    }
    #[test]
    fn test_encoding_decoding2() {
        let encoded = hex::decode("f8194e480000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000").unwrap();
        println!("{:?}", &encoded);
        let encoded_bytes = Bytes::from(encoded);

        let decoded_result =
            SolidityABI::<<GreetingCall as Deref>::Target>::decode(&encoded_bytes, 4);
        println!("decoded result: {:?}", decoded_result.is_ok());
        let message = match decoded_result {
            Ok(decoded) => decoded.0,
            Err(_) => {
                panic!("failed to decode input");
            }
        };
        println!("message > ~ : {:?}", message);
    }

    #[test]
    fn test_contract_works() {
        let b = Bytes::from("Hello, World!!".as_bytes());
        let a = Address::repeat_byte(0xAA);

        let greeting_call = GreetingCall::new((b.clone(), a.clone()));

        let input = greeting_call.encode();
        sol!(
            function buying(bytes message, address caller);
        );

        let buying_call = buyingCall {
            message: b.clone(),
            caller: a.clone(),
        };

        let byuing_call_input = buying_call.abi_encode();
        println!("bying_call_input: {:?}", hex::encode(&byuing_call_input));

        let expected_input = "0x00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000040000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000";

        println!("encoded: {:?}", hex::encode(&input));
        assert_eq!(hex::encode(&input[4..]), expected_input);

        // let expected_encoded =
        // "0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000"
        // ;

        // assert_eq!(hex::encode(&input[4..]), expected_encoded);

        println!("Decoding...");

        let decoded = SolidityABI::<Bytes>::decode(&input, 0).unwrap();

        println!(
            "decoded: {:?}",
            String::from_utf8(decoded.to_vec()).unwrap()
        );

        let sdk = TestingContext::empty().with_input(input);
        let mut router = ROUTER::new(JournalState::empty(sdk.clone()));
        router.deploy();
        router.main();

        let encoded_output = &sdk.take_output();

        let output = GreetingReturn::decode(&encoded_output.as_slice()).unwrap();
        // TODO: uncomment next line
        // assert_eq!(output.0 .0, msg);
    }
}

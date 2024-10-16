#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use bytes::{Buf, BytesMut};
use codec2::{encoder::SolidityABI, error::CodecError};
use core::ops::Deref;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature, Contract},
    Bytes,
    SharedAPI,
};

#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn greeting(&self, message: Bytes) -> Bytes;
    // fn custom_greeting(&self, message: Bytes) -> Bytes;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    // #[signature("greeting(bytes)")] // 0xf8194e48
    fn greeting(&self, message: Bytes) -> Bytes {
        message
    }

    // #[signature("customGreeting(string)")]
    // fn custom_greeting(&self, message: String) -> String {
    //     message
    // }
}

// use byteorder::{ByteOrder, LittleEndian};
// use bytes::{Buf, BytesMut};
// use codec2::{
//     encoder::{align_up, read_u32_aligned, write_u32_aligned, Encoder},
//     error::CodecError,
// };

// // type GreetingCall = (Bytes,);
// // type GreetingReturn = (Bytes,);

// // mod greeting_call {
// //     use super::Bytes;

// //     pub const SELECTOR: [u8; 4] = [248, 25, 78, 72];
// //     pub const SIGNATURE: &str = "greeting(bytes)";

// //     // Other associated functions can be added here if needed
// //     pub fn new(message: Bytes) -> crate::GreetingCall {
// //         (message,)
// //     }
// // }

// pub type GreetingCallArgs = (Bytes,);
// pub struct GreetingCall(GreetingCallArgs);

// impl GreetingCall {
//     const SELECTOR: [u8; 4] = [248, 25, 78, 72];
//     const SIGNATURE: &'static str = "greeting(bytes)";

//     fn new(args: GreetingCallArgs) -> Self {
//         Self(args)
//     }

//     fn encode(&self) -> Bytes {
//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&self.0, &mut buf, 0).unwrap();
//         let encoded_args = buf.freeze();

//         Self::SELECTOR.iter().copied().chain(encoded_args).collect()
//     }

//     pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
//         let chunk = buf.chunk();
//         if chunk.len() < 4 {
//             return Err(CodecError::Decoding(
//                 codec2::error::DecodingError::BufferTooSmall {
//                     expected: 4,
//                     found: chunk.len(),
//                     msg: "buf too small to read fn selector".to_string(),
//                 },
//             ));
//         }

//         let selector: [u8; 4] = chunk[..4].try_into().unwrap();
//         if selector != Self::SELECTOR {
//             return Err(CodecError::Decoding(
//                 codec2::error::DecodingError::InvalidSelector {
//                     expected: Self::SELECTOR,
//                     found: selector,
//                 },
//             ));
//         }

//         let args = SolidityABI::<GreetingCallArgs>::decode(&&chunk[4..], 0)?;
//         Ok(Self(args))
//     }
// }

// impl Deref for GreetingCall {
//     type Target = GreetingCallArgs;

//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

// pub type GreetingCallTarget = <GreetingCall as Deref>::Target;

// pub type GreetingReturnArgs = (Bytes,);
// pub struct GreetingReturn(GreetingReturnArgs);

// impl GreetingReturn {
//     fn new(args: GreetingReturnArgs) -> Self {
//         Self(args)
//     }

//     fn encode(&self) -> Bytes {
//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&self.0, &mut buf, 0).unwrap();
//         buf.freeze().into()
//     }

//     pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
//         let args = SolidityABI::<GreetingReturnArgs>::decode(buf, 0)?;
//         Ok(Self(args))
//     }
// }

// impl Deref for GreetingReturn {
//     type Target = GreetingReturnArgs;

//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

// pub type GreetingReturnTarget = <GreetingReturn as Deref>::Target;

impl<SDK: SharedAPI> ROUTER<SDK> {
    pub fn main(&mut self) {
        println!("op main");
        let input_size = self.sdk.input_size();
        if input_size < 4 {
            panic!("input too short, cannot extract selector");
        };
        let mut full_input = fluentbase_sdk::alloc_slice(input_size as usize);

        self.sdk.read(&mut full_input, 0);
        let (selector, data_input) = full_input.split_at(4);

        let selector: [u8; 4] = selector.try_into().expect("Selector should be 4 bytes");

        println!("input: {:?}", data_input);
        match selector {
            GreetingCall::SELECTOR => {
                let decoded_result = SolidityABI::<GreetingCallArgs>::decode(&data_input, 0);

                let message = match decoded_result {
                    Ok(decoded) => decoded.0,
                    Err(_) => {
                        panic!("failed to decode input");
                    }
                };

                let output = self.greeting(message);
                let encoded_output = GreetingReturn::new((output,)).encode();

                self.sdk.write(&encoded_output);
            }
            _ => {
                panic!("unknown method");
            }
        }
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
        let msg = Bytes::from("Hello, World!!".as_bytes());
        let selector: [u8; 4] = [248, 25, 78, 72];
        let greeting_call = GreetingCall::new((msg.clone(),));

        let input = greeting_call.encode();
        let expected_encoded = "0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000";

        assert_eq!(hex::encode(&input[4..]), expected_encoded);

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
        assert_eq!(output.0 .0, msg);
    }
}

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use bytes::{Buf, BufMut, BytesMut};
use codec2::{
    encoder::{FluentABI, SolidityABI},
    error::CodecError,
};
use core::ops::Deref;
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

#[router(mode = "codec")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[function_id("greeting(bytes,address)")] // 0xf8194e48
    fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
        message
    }
}

// // Recursive expansion of router macro
// // ====================================

// #[allow(unused_imports)]
// use fluentbase_sdk::derive::function_id;
// impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
//     fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
//         message
//     }
// }
// pub use codec2::encoder::Encoder;
// pub type GreetingCallArgs = (Bytes, Address);
// pub struct GreetingCall(GreetingCallArgs);

// impl GreetingCall {
//     pub const SELECTOR: [u8; 4] = [238u8, 148u8, 111u8, 40u8];
//     pub const SIGNATURE: &'static str = "greeting(bytes,address)";
//     pub fn new(args: GreetingCallArgs) -> Self {
//         Self(args)
//     }
//     pub fn encode(&self) -> Bytes {
//         let mut buf = BytesMut::new();
//         let values = (self.0.clone().0, self.0.clone().1);
//         println!("values: {:?}", &values);
//         FluentABI::encode(&values, &mut buf, 0).unwrap();
//         let encoded_args = buf.freeze();

//         println!("encoded_args: {:?}", encoded_args);
//         let clean_args = if FluentABI::<GreetingCallArgs>::is_dynamic() {
//             encoded_args[4..].to_vec()
//         } else {
//             encoded_args.to_vec()
//         };
//         println!("clean_args: {:?}", &clean_args);
//         Self::SELECTOR.iter().copied().chain(clean_args).collect()
//     }
//     pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
//         println!("op decode");
//         println!("buf: {:?}", buf.chunk());
//         let dynamic_offset = if FluentABI::<GreetingCallArgs>::is_dynamic() {
//             (4 as u32).to_le_bytes().to_vec()
//         } else {
//             [].to_vec()
//         };
//         let mut combined_buf = BytesMut::new();
//         combined_buf.put_slice(&dynamic_offset);
//         combined_buf.put_slice(buf.chunk());
//         let combined_bytes = combined_buf.freeze();
//         println!("combined_bytes: {:02x?}", &combined_bytes.chunk());
//         let args = FluentABI::<GreetingCallArgs>::decode(&combined_bytes, 0)?;
//         println!("args: {:?}", &args);
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
//     pub fn new(args: GreetingReturnArgs) -> Self {
//         Self(args)
//     }
//     pub fn encode(&self) -> Bytes {
//         let mut buf = BytesMut::new();
//         FluentABI::encode(&self.0, &mut buf, 0).unwrap();
//         buf.freeze().into()
//     }
//     pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
//         let args = FluentABI::<GreetingReturnArgs>::decode(buf, 0)?;
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
// impl<SDK: SharedAPI> ROUTER<SDK> {
//     pub fn main(&mut self) {
//         let input_size = self.sdk.input_size();
//         if input_size < 4 {
//             {
//                 panic!("input too short, cannot extract selector");
//             };
//         };
//         let mut full_input = fluentbase_sdk::alloc_slice(input_size as usize);
//         self.sdk.read(&mut full_input, 0);
//         let (selector, data_input) = full_input.split_at(4);
//         match [selector[0], selector[1], selector[2], selector[3]] {
//             GreetingCall::SELECTOR => {
//                 let (message, caller) = match GreetingCall::decode(&data_input) {
//                     Ok(decoded) => (decoded.0 .0, decoded.0 .1),
//                     Err(err) => {
//                         {
//                             panic!("failed to decode input: {:?}", err);
//                         };
//                     }
//                 };
//                 let output = self.greeting(message, caller);
//                 let encoded_output = GreetingReturn::new((output,)).encode();
//                 self.sdk.write(&encoded_output);
//             }
//             _ => {
//                 panic!("unknown method");
//             }
//         }
//     }
// }

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
        assert_eq!(output.0 .0, b);
    }
}

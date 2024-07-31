#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    codec::{Codec, Encoder},
    derive::{client, router, signature, Contract},
    Bytes,
    SovereignAPI,
};

#[derive(Default, Codec)]
pub struct HelloWorldInput {}

#[derive(Default, Codec)]
pub struct HelloWorldOutput {
    message: Bytes,
}

// #[client(mode = "codec")]
pub trait PrecompileAPI {
    #[signature("hello_world()")]
    fn hello_world(&self, input: HelloWorldInput) -> HelloWorldOutput;
}

#[derive(Contract)]
pub struct PRECOMPILE<SDK> {
    sdk: SDK,
}

#[router(mode = "codec")]
impl<SDK: SovereignAPI> PrecompileAPI for PRECOMPILE<SDK> {
    #[signature("hello_world()")]
    fn hello_world(&self, _input: HelloWorldInput) -> HelloWorldOutput {
        HelloWorldOutput {
            message: "Hello, World".into(),
        }
    }
}

basic_entrypoint!(PRECOMPILE);

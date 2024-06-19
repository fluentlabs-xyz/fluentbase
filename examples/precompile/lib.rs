#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    codec::{Codec, Encoder},
    derive::{client, router, signature, Contract},
    AccountManager,
    Bytes,
    ContextReader,
    SharedAPI,
};

#[derive(Default, Codec)]
pub struct HelloWorldInput {}

#[derive(Default, Codec)]
pub struct HelloWorldOutput {
    message: Bytes,
}

#[client(mode = "codec")]
pub trait PrecompileAPI {
    #[signature("hello_world()")]
    fn hello_world(&self, input: HelloWorldInput) -> HelloWorldOutput;
}

#[derive(Contract)]
pub struct PRECOMPILE<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

#[router(mode = "codec")]
impl<'a, CR: ContextReader, AM: AccountManager> PrecompileAPI for PRECOMPILE<'a, CR, AM> {
    #[signature("hello_world()")]
    fn hello_world(&self, _input: HelloWorldInput) -> HelloWorldOutput {
        HelloWorldOutput {
            message: "Hello, World".into(),
        }
    }
}

basic_entrypoint!(
    PRECOMPILE<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);

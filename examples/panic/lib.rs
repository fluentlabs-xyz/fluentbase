#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, ContextReader, SharedAPI};

#[derive(Contract)]
struct PANIC<CTX, SDK> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SharedAPI> PANIC<CTX, SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // write "Hello, World" message into output
        panic!("it is panic time")
    }
}

basic_entrypoint!(PANIC);

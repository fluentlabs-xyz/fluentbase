#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, NativeAPI, SharedAPI};

#[derive(Contract)]
struct PANIC<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> PANIC<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // write "Hello, World" message into output
        panic!("it is panic time")
    }
}

basic_entrypoint!(PANIC);

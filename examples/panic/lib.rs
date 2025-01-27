#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(unused)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, SharedAPI};

#[derive(Contract)]
struct PANIC<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> PANIC<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // panic with some message
        self.sdk.panic("it's panic time");
        // panic!("it is panic time")
    }
}

basic_entrypoint!(PANIC);

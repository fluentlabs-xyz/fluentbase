#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
};

#[derive(Contract)]
struct MyContract<SDK> {
    sdk: SDK,
}

#[router]
impl<SDK: SharedAPI> MyContract<SDK> {
    fn private_method(&self, a: u32) -> u64 {
        a as u64
    }
}

basic_entrypoint!(MyContract);

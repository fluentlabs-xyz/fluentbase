#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    SharedAPI,
};

#[derive(Contract)]
struct MyContract<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> MyContract<SDK> {
    fn method_a(&self, a: u32, b: u64) -> u128 {
        a as u128 + b as u128
    }

    fn method_b(&self, s: String) -> String {
        s + " processed"
    }

    fn method_c(&self, a: u8, b: bool) -> (u8, bool) {
        (a, !b)
    }
}

impl<SDK: SharedAPI> MyContract<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(MyContract);

fn main() {}

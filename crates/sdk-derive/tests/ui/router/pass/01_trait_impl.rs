#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract},
    SharedAPI,
};

#[derive(Contract)]
struct MyContract<SDK> {
    sdk: SDK,
}

pub trait ComplexAPI {
    fn method_a(&self, a: u32, b: u64) -> u64;
    fn method_b(&self, s: String) -> String;
    fn method_c(&self, a: u8, b: bool) -> (u8, bool);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ComplexAPI for MyContract<SDK> {
    fn method_a(&self, a: u32, b: u64) -> u64 {
        a as u64 + b
    }

    #[function_id("customMethodName(string)", validate(false))]
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

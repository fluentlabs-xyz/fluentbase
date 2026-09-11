#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    SharedAPI, U256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn get_value(&self) -> U256;

    // Handles every selector the router does not know, and inputs shorter than a selector
    fn fallback(&self);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    #[function_id("getValue()", validate(true))]
    fn get_value(&self) -> U256 {
        U256::from(42)
    }

    fn fallback(&self) {}
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(App);

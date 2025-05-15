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
struct App<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    // Method expects a String but function_id declares an address parameter
    fn greeting(&self, message: String) -> String;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    // This should fail because the function ID specifies address type
    // but the method takes a String
    #[function_id("greeting(address)", validate(true))]
    fn greeting(&self, message: String) -> String {
        message
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(App);

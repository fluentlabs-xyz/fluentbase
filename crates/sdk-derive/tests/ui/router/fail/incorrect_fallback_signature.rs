#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
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
    // Fallback must have no parameters and no return value
    fn fallback(&self, data: u64) -> bool;

    // Regular method for the router to have something to process
    fn regular_method(&self);
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for App<SDK> {
    fn fallback(&self, data: u64) -> bool {
        // Should cause an error as fallback cannot have parameters or return value
        data > 10
    }

    #[function_id("regularMethod()", validate(true))]
    fn regular_method(&self) {
        // Normal method
    }
}

impl<SDK: SharedAPI> App<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(App);

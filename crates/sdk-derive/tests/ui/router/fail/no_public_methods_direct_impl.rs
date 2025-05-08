#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

// Direct implementation without using public methods
#[router(mode = "solidity")]
impl<SDK: SharedAPI> App<SDK> {
    // Private method - this should cause an error since in direct implementation
    // only public methods are included in routing
    fn private_method(&self, value: u32) -> u32 {
        value
    }

    // Another private method
    fn another_private_method(&self, message: String) -> String {
        message
    }

    // Regular deployment method - not public but special case
    fn deploy(&self) {}
}

basic_entrypoint!(App);

fn main() {}

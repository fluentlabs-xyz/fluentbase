#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, SharedAPI};

#[derive(Default)]
struct PANIC;

impl PANIC {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        // write "Hello, World" message into output
        panic!("it is panic time")
    }
}

basic_entrypoint!(PANIC);

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{address, entrypoint, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // Write any output into
    sdk.write([0x01]);
    let balance = sdk.balance(&address!("0x0000000000000000000000000000000000000001"));
    core::hint::black_box(balance);
}

entrypoint!(main_entry);

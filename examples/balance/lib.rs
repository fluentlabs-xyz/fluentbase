#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{address, entrypoint, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let balance = sdk.balance(&address!("0x0000000000000000000000000000000000000001"));
    let result = balance.unwrap();
    let result = result.to_le_bytes::<32>();
    sdk.write(result);
}

entrypoint!(main_entry);

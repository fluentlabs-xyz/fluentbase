#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
use fluentbase_sdk::{entrypoint, SharedAPI};

pub fn main_entry(_sdk: impl SharedAPI) {
    todo!("not implemented")
}

entrypoint!(main_entry);

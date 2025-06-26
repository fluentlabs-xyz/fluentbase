#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate core;

#[allow(unused_imports)]
use fluentbase_eip2935::entry::main_entry;
use fluentbase_sdk::entrypoint;

entrypoint!(main_entry);

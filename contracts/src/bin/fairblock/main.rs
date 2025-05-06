#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;
#[allow(dead_code)]
fn call(_sdk: impl SharedAPI) {
    // check "main.go" instead
}

use fluentbase_sdk::{func_entrypoint, SharedAPI};
func_entrypoint!(call);

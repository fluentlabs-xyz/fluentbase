#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
use fluentbase_sdk::{func_entrypoint, SharedAPI};

fn call(_sdk: impl SharedAPI) {
    todo!("not implemented")
}

func_entrypoint!(call);

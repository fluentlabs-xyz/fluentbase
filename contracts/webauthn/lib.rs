#![cfg_attr(target_arch = "wasm32", no_std)]
use fluentbase_sdk::{func_entrypoint, SharedAPI};

pub fn main(_sdk: impl SharedAPI) {
    todo!("not implemented")
}

func_entrypoint!(main);

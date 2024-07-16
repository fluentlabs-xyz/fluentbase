#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::router, SharedAPI};

pub trait TilesAPI {
    fn check(&self);
}

#[derive(Default)]
struct TILES<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> TilesAPI for TILES<SDK> {
    fn check(&self) {}
}

impl<SDK: SharedAPI> TILES<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(TILES);

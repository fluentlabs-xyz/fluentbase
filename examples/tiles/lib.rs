#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::router, SharedAPI};

pub trait TilesAPI {
    fn check<SDK: SharedAPI>(&self);
}

#[derive(Default)]
struct TILES;

#[router(mode = "solidity")]
impl TilesAPI for TILES {
    fn check<SDK: SharedAPI>(&self) {}
}

impl TILES {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(TILES);

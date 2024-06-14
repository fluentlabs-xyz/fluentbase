#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::router, SharedAPI, Bytes};

#[derive(Default)]
struct TILES;

#[router(mode = "solidity")]
impl TILES {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    pub fn check(&self, moves: Bytes) {}
}

basic_entrypoint!(TILES);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_works() {
        let greeting = TILES::default();
        greeting.deploy::<LowLevelSDK>();
        greeting.main::<LowLevelSDK>();
        let test_output = LowLevelSDK::get_test_output();
        assert_eq!(&test_output, "Hello, World".as_bytes());
    }
}

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, SharedAPI};
use shakmaty::{Chess, Position};

#[derive(Default)]
struct SHAKMATY;

impl SHAKMATY {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        let pos = Chess::default();
        let legals = pos.legal_moves();
        assert_eq!(legals.len(), 20);
    }
}

basic_entrypoint!(SHAKMATY);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_works() {
        let shakmaty = SHAKMATY::default();
        shakmaty.deploy::<LowLevelSDK>();
        shakmaty.main::<LowLevelSDK>();
    }
}

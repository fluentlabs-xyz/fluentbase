#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, SharedAPI};
use shakmaty::{Chess, Position};

#[derive(Contract)]
struct SHAKMATY<SDK> {
    #[allow(unused)]
    sdk: SDK,
}

impl<SDK: SharedAPI> SHAKMATY<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        let pos = Chess::default();
        let legals = pos.legal_moves();
        assert_eq!(legals.len(), 20);
    }
}

basic_entrypoint!(SHAKMATY);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let sdk = JournalState::empty(TestingContext::new());
        let shakmaty = SHAKMATY::new(sdk);
        shakmaty.deploy();
        shakmaty.main();
    }
}

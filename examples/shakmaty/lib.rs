#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, ContextReader, SharedAPI};
use shakmaty::{Chess, Position};

#[derive(Contract)]
struct SHAKMATY<CTX, SDK> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SharedAPI> SHAKMATY<CTX, SDK> {
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
    use fluentbase_sdk::{runtime::TestingContext, ContractInput};

    #[test]
    fn test_contract_works() {
        let ctx = ContractInput::default();
        let sdk = TestingContext::new();
        let shakmaty = SHAKMATY::new(ctx, sdk);
        shakmaty.deploy();
        shakmaty.main();
    }
}

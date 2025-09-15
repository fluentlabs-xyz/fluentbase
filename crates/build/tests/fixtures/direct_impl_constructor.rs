// Test case: Direct implementation with constructor
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    derive::{router, Codec, Contract},
    Address,
    SharedAPI,
    U256,
};

#[derive(Contract)]
struct SimpleToken<SDK> {
    sdk: SDK,
}

#[derive(Debug, Clone, Codec)]
pub struct TokenConfig {
    pub name: String,
    pub symbol: String,
    pub total_supply: U256,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SimpleToken<SDK> {
    // Constructor with parameters
    pub fn constructor(&mut self, owner: Address, config: TokenConfig) {
        // Implementation details don't matter for ABI generation
    }

    pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
        true
    }

    pub fn balance_of(&self, account: Address) -> U256 {
        U256::from(0)
    }

    pub fn total_supply(&self) -> U256 {
        U256::from(0)
    }
}

basic_entrypoint!(SimpleToken);

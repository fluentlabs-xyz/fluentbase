// Test case: Trait implementation with constructor
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    derive::{constructor, router, Contract},
    Address, SharedAPI, U256,
};

#[derive(Contract)]
struct Governance<SDK> {
    sdk: SDK,
}

pub trait GovernanceAPI {
    fn propose(&mut self, target: Address, value: U256) -> U256;
    fn vote(&mut self, proposal_id: U256, support: bool) -> bool;
    fn execute(&mut self, proposal_id: U256) -> bool;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> GovernanceAPI for Governance<SDK> {
    fn propose(&mut self, target: Address, value: U256) -> U256 {
        U256::from(1)
    }

    fn vote(&mut self, proposal_id: U256, support: bool) -> bool {
        true
    }

    fn execute(&mut self, proposal_id: U256) -> bool {
        true
    }
}

#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> Governance<SDK> {
    pub fn constructor(&mut self, admin: Address, voting_delay: U256, voting_period: U256) {
        // Implementation details
    }
}

basic_entrypoint!(SimpleToken);

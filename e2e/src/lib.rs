#![allow(soft_unstable)]

extern crate alloc;
extern crate core;

use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{GenesisContract, PRECOMPILE_WASM_RUNTIME};
use fluentbase_testing::EvmTestingContext;

#[cfg(test)]
mod blockhash;
#[cfg(test)]
mod bridge;
#[cfg(test)]
mod builtins;
#[cfg(test)]
mod constructor;
#[cfg(test)]
mod deployer;
#[cfg(test)]
mod eip2935;
#[cfg(test)]
mod evm;
#[cfg(test)]
mod gas;
#[cfg(test)]
mod helpers;
#[cfg(test)]
mod multicall;
// #[cfg(test)]
// mod nitro;
#[cfg(test)]
mod router;
#[cfg(test)]
mod stateless;
#[cfg(all(test, feature = "svm"))]
pub mod svm;
// #[cfg(test)]
// mod universal_token;
#[cfg(test)]
mod ddos;
// #[cfg(test)]
// mod erc20;
#[cfg(test)]
mod bench;
#[cfg(test)]
mod update_account;
#[cfg(test)]
mod wasm;

pub trait EvmTestingContextWithGenesis {
    fn with_full_genesis(self) -> Self;

    fn with_minimal_genesis(self) -> Self;
}

impl EvmTestingContextWithGenesis for EvmTestingContext {
    fn with_full_genesis(self) -> EvmTestingContext {
        let contracts: Vec<GenesisContract> = GENESIS_CONTRACTS_BY_ADDRESS
            .iter()
            .map(|(_k, v)| v.clone())
            .collect();
        self.with_contracts(&contracts)
    }

    fn with_minimal_genesis(self) -> EvmTestingContext {
        let wasm_runtime = GENESIS_CONTRACTS_BY_ADDRESS
            .get(&PRECOMPILE_WASM_RUNTIME)
            .unwrap()
            .clone();
        self.with_contracts(&[wasm_runtime])
    }
}

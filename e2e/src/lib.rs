#![allow(soft_unstable)]
#![allow(unused)]

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
// SECURITY: Multicall tests use calldata-based precompile dispatch (testnet-only).
// See detailed vulnerability explanation in frame_init() handler.
#[cfg(feature = "fluent-testnet")]
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
#[cfg(test)]
mod universal_token;
// Testnet-only: Runtime upgrade functionality. See frame_init() for details.
#[cfg(test)]
mod bench;
#[cfg(feature = "fluent-testnet")]
#[cfg(test)]
mod ddos;
#[cfg(test)]
mod exec_input;
mod oauth2;
#[cfg(test)]
mod oom;
#[cfg(feature = "fluent-testnet")]
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

#![allow(soft_unstable)]

extern crate alloc;
extern crate core;

use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_testing::EvmTestingContext;
use fluentbase_types::{GenesisContract, PRECOMPILE_WASM_RUNTIME};

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
mod erc20;
#[cfg(test)]
mod evm;
#[cfg(test)]
mod gas;
#[cfg(test)]
mod multicall;
#[cfg(test)]
mod nitro;
#[cfg(test)]
mod router;
#[cfg(test)]
mod stateless;
#[cfg(all(test, feature = "enable-svm"))]
pub mod svm;
#[cfg(test)]
mod update_account;
#[cfg(test)]
mod wasm;

pub const EXAMPLE_ABI_SOLIDITY: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_ABI_SOLIDITY.wasm_bytecode;
pub const EXAMPLE_CHECKMATE: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_CHECKMATE.wasm_bytecode;
pub const EXAMPLE_CLIENT_SOLIDITY: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_CLIENT_SOLIDITY.wasm_bytecode;
pub const EXAMPLE_CONSTRUCTOR_PARAMS: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_CONSTRUCTOR_PARAMS.wasm_bytecode;
pub const EXAMPLE_ERC20: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_ERC20.wasm_bytecode;
pub const EXAMPLE_GREETING: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode;
pub const EXAMPLE_JSON: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_JSON.wasm_bytecode;
pub const EXAMPLE_KECCAK256: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_KECCAK.wasm_bytecode;
pub const EXAMPLE_PANIC: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_PANIC.wasm_bytecode;
pub const EXAMPLE_ROUTER_SOLIDITY: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_ROUTER_SOLIDITY.wasm_bytecode;
pub const EXAMPLE_RWASM: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_RWASM.wasm_bytecode;
pub const EXAMPLE_SECP256K1: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_SECP256K1.wasm_bytecode;
pub const EXAMPLE_SIMPLE_STORAGE: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_SIMPLE_STORAGE.wasm_bytecode;
pub const EXAMPLE_STORAGE: &[u8] = fluentbase_contracts::FLUENTBASE_EXAMPLES_STORAGE.wasm_bytecode;
pub const EXAMPLE_TINY_KECCAK256: &[u8] =
    fluentbase_contracts::FLUENTBASE_EXAMPLES_TINY_KECCAK.wasm_bytecode;

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

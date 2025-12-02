mod evm;
mod host;
mod store;

pub use evm::*;
pub use fluentbase_sdk::include_this_wasm;
// use fluentbase_sdk::{GenesisContract, PRECOMPILE_WASM_RUNTIME};
pub use host::*;
pub use store::*;

use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_sdk::{GenesisContract, PRECOMPILE_WASM_RUNTIME};

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

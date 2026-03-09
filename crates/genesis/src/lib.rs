//! Genesis helpers and embedded build outputs for Fluentbase system contracts.
pub use alloy_genesis::Genesis;
use fluentbase_sdk::{Address, Bytes, GenesisContract, B256};
use lazy_static::lazy_static;
use std::collections::HashMap;

pub fn local_genesis_from_file() -> Genesis {
    let json_file = include_str!("../genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

include!(concat!(env!("OUT_DIR"), "/build_output.rs"));

lazy_static! {
    pub static ref GENESIS_CONTRACTS_BY_ADDRESS: HashMap<Address, GenesisContract> = {
        let mut map = HashMap::new();
        for build_output in BUILD_OUTPUTS {
            let contract = GenesisContract {
                name: build_output.name,
                rwasm_bytecode: Bytes::from_static(build_output.rwasm_bytecode),
                rwasm_bytecode_hash: B256::from_slice(&build_output.rwasm_bytecode_hash),
                address: Address::from(&build_output.address),
            };
            map.insert(contract.address, contract);
        }
        map
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::PRECOMPILE_CREATE2_FACTORY;

    #[test]
    fn genesis_contains_create2_factory() {
        let contract = GENESIS_CONTRACTS_BY_ADDRESS
            .get(&PRECOMPILE_CREATE2_FACTORY)
            .expect("missing create2 factory in genesis");
        assert_eq!(contract.name, "fluentbase_create2_factory");
        assert!(!contract.rwasm_bytecode.is_empty());
    }
}

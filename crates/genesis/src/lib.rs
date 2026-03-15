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
    use crate::local_genesis_from_file;
    use fluentbase_revm::revm::primitives::address;
    use fluentbase_sdk::{Address, U256};

    #[test]
    fn test_ecosystem_faucet_and_bridge_have_enough_funds() {
        const BRIDGE_DEPLOYER: Address = address!("0x482582979C9125abAb5a06F0E196E8F4015bF77A");
        const ECOSYSTEM_FAUCET: Address = address!("0xb58A6bdEB3387C87d55b7baE800f3C816f35DC34");
        let genesis = local_genesis_from_file();
        assert_eq!(
            genesis.alloc.get(&BRIDGE_DEPLOYER).unwrap().balance,
            U256::from(1_000000000000000000u128),
        );
        assert_eq!(
            genesis.alloc.get(&ECOSYSTEM_FAUCET).unwrap().balance,
            U256::from(9_000000000000000000u128),
        );
    }
}

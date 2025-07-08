pub use alloy_genesis::Genesis;
use fluentbase_types::{Address, Bytes, B256};
use lazy_static::lazy_static;
use std::collections::HashMap;

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub struct GenesisContract {
    pub name: &'static str,
    pub rwasm_bytecode: Bytes,
    pub rwasm_bytecode_hash: B256,
    pub address: Address,
}

include!(concat!(env!("OUT_DIR"), "/build_output.rs"));

lazy_static! {
    pub static ref GENESIS_CONTRACTS_BY_ADDRESS: HashMap<Address, GenesisContract> = {
        let mut map = HashMap::new();
        for build in BUILD_OUTPUTS {
            let contract = GenesisContract {
                name: build.name,
                rwasm_bytecode: Bytes::from_static(build.rwasm_bytecode),
                rwasm_bytecode_hash: B256::from_slice(&build.rwasm_bytecode_hash),
                address: Address::from(&build.address),
            };
            map.insert(contract.address.clone(), contract);
        }
        map
    };
}

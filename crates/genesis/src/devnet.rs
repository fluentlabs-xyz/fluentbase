use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::address;
use std::collections::HashMap;

fn devnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 1337,
        ..Default::default()
    }
}

pub const ECL_CONTRACT_ADDRESS: fluentbase_types::Address =
    address!("5200000000000000000000000000000000000001");
pub const WCL_CONTRACT_ADDRESS: fluentbase_types::Address =
    address!("5200000000000000000000000000000000000002");
pub const PRECOMPILE_BLAKE2_ADDRESS: fluentbase_types::Address =
    address!("0000000000000000000000000000000000000001");

pub fn devnet_genesis() -> Genesis {
    let mut alloc = HashMap::new();
    alloc.insert(
        ECL_CONTRACT_ADDRESS,
        GenesisAccount {
            code: Some(include_bytes!("../../contracts/assets/ecl_contract.rwasm").into()),
            ..Default::default()
        },
    );
    alloc.insert(
        WCL_CONTRACT_ADDRESS,
        GenesisAccount {
            code: Some(include_bytes!("../../contracts/assets/wcl_contract.rwasm").into()),
            ..Default::default()
        },
    );
    alloc.insert(
        PRECOMPILE_BLAKE2_ADDRESS,
        GenesisAccount {
            code: Some(include_bytes!("../../contracts/assets/precompile_blake2.rwasm").into()),
            ..Default::default()
        },
    );
    Genesis {
        config: devnet_chain_config(),
        nonce: 0,
        timestamp: 0,
        extra_data: Default::default(),
        gas_limit: 0,
        difficulty: Default::default(),
        mix_hash: Default::default(),
        coinbase: Default::default(),
        alloc,
        base_fee_per_gas: None,
        excess_blob_gas: None,
        blob_gas_used: None,
        number: None,
    }
}

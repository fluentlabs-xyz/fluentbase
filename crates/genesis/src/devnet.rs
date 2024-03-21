use crate::{
    ChainConfig,
    Genesis,
    GenesisAccount,
    ECL_CONTRACT_ADDRESS,
    EXAMPLE_GREETING_ADDRESS,
    PRECOMPILE_BLAKE2_ADDRESS,
    WCL_CONTRACT_ADDRESS,
};
use fluentbase_types::Bytes;
use std::collections::HashMap;

pub fn devnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 1337,
        ..Default::default()
    }
}

pub fn devnet_genesis() -> Genesis {
    let mut alloc = HashMap::new();
    macro_rules! enable_rwasm_contract {
        ($addr:ident, $file_path:literal) => {
            alloc.insert(
                $addr,
                GenesisAccount {
                    code: Some(Bytes::from(include_bytes!($file_path))),
                    ..Default::default()
                },
            );
        };
    }
    enable_rwasm_contract!(
        ECL_CONTRACT_ADDRESS,
        "../../contracts/assets/ecl_contract.rwasm"
    );
    enable_rwasm_contract!(
        WCL_CONTRACT_ADDRESS,
        "../../contracts/assets/wcl_contract.rwasm"
    );
    enable_rwasm_contract!(
        PRECOMPILE_BLAKE2_ADDRESS,
        "../../contracts/assets/precompile_blake2.rwasm"
    );
    enable_rwasm_contract!(
        EXAMPLE_GREETING_ADDRESS,
        "../../../examples/bin/greeting.rwasm"
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

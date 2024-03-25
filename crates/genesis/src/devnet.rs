use crate::{ChainConfig, Genesis, GenesisAccount, EXAMPLE_GREETING_ADDRESS};
use fluentbase_core::consts::{
    ECL_CONTRACT_ADDRESS,
    PRECOMPILE_BLAKE2_ADDRESS,
    PRECOMPILE_IDENTITY_ADDRESS,
    PRECOMPILE_MODEXP_ADDRESS,
    PRECOMPILE_SECP256K1_ADDRESS,
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
    // enable_rwasm_contract!(
    //     PRECOMPILE_BN128_ADDRESS,
    //     "../../contracts/assets/precompile_bn128.rwasm"
    // );
    enable_rwasm_contract!(
        PRECOMPILE_IDENTITY_ADDRESS,
        "../../contracts/assets/precompile_identity.rwasm"
    );
    // enable_rwasm_contract!(
    //     PRECOMPILE_KZG_POINT_EVALUATION_ADDRESS,
    //     "../../contracts/assets/precompile_kzg_point_evaluation.rwasm"
    // );
    enable_rwasm_contract!(
        PRECOMPILE_MODEXP_ADDRESS,
        "../../contracts/assets/precompile_modexp.rwasm"
    );
    enable_rwasm_contract!(
        PRECOMPILE_SECP256K1_ADDRESS,
        "../../contracts/assets/precompile_secp256k1.rwasm"
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

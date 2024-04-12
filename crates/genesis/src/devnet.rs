use crate::{ChainConfig, Genesis, GenesisAccount, EXAMPLE_GREETING_ADDRESS};
use fluentbase_core::consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{b256, Bytes, B256};
use std::collections::BTreeMap;

pub fn devnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 1337,
        homestead_block: Some(0u64),
        dao_fork_block: Some(0u64),
        dao_fork_support: false,
        eip150_block: Some(0u64),
        eip150_hash: None,
        eip155_block: Some(0u64),
        eip158_block: Some(0u64),
        byzantium_block: Some(0u64),
        constantinople_block: Some(0u64),
        petersburg_block: Some(0u64),
        istanbul_block: Some(0u64),
        muir_glacier_block: Some(0u64),
        berlin_block: Some(0u64),
        london_block: Some(0u64),
        arrow_glacier_block: Some(0u64),
        gray_glacier_block: Some(0u64),
        merge_netsplit_block: Some(0u64),
        shanghai_time: Some(0u64),
        cancun_time: Some(0u64),
        terminal_total_difficulty: None,
        terminal_total_difficulty_passed: false,
        ethash: None,
        clique: None,
    }
}

pub const POSEIDON_HASH_KEY: B256 =
    b256!("0000000000000000000000000000000000000000000000000000000000000001");

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis() -> Genesis {
    let mut alloc = BTreeMap::new();
    macro_rules! enable_rwasm_contract {
        ($addr:ident, $file_path:literal) => {{
            let bytecode = Bytes::from(include_bytes!($file_path));
            let bytecode_hash = poseidon_hash(&bytecode);
            alloc.insert(
                $addr,
                GenesisAccount {
                    code: Some(bytecode),
                    storage: Some(BTreeMap::from([(POSEIDON_HASH_KEY, bytecode_hash.into())])),
                    ..Default::default()
                },
            );
        }};
    }
    enable_rwasm_contract!(
        ECL_CONTRACT_ADDRESS,
        "../../contracts/assets/ecl_contract.rwasm"
    );
    enable_rwasm_contract!(
        WCL_CONTRACT_ADDRESS,
        "../../contracts/assets/wcl_contract.rwasm"
    );
    // enable_rwasm_contract!(
    //     PRECOMPILE_BLAKE2_ADDRESS,
    //     "../../contracts/assets/precompile_blake2.rwasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_BN128_ADDRESS,
    //     "../../contracts/assets/precompile_bn128.rwasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_IDENTITY_ADDRESS,
    //     "../../contracts/assets/precompile_identity.rwasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_KZG_POINT_EVALUATION_ADDRESS,
    //     "../../contracts/assets/precompile_kzg_point_evaluation.rwasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_MODEXP_ADDRESS,
    //     "../../contracts/assets/precompile_modexp.rwasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_SECP256K1_ADDRESS,
    //     "../../contracts/assets/precompile_secp256k1.rwasm"
    // );
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

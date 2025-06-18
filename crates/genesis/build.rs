use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::{
    address,
    Address,
    Bytes,
    GenesisContractBuildOutput,
    B256,
    DEVELOPER_PREVIEW_CHAIN_ID,
    U256,
};
use std::{collections::BTreeMap, env, fs::File, io::Write, path::PathBuf};

#[rustfmt::skip]
const GENESIS_CONTRACTS: &[(Address, GenesisContractBuildOutput)] = &[
    (fluentbase_types::PRECOMPILE_BIG_MODEXP, fluentbase_contracts_modexp::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLAKE2F, fluentbase_contracts_blake2f::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_ADD, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_MUL, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_PAIR, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_ERC20, fluentbase_contracts_erc20::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_EVM_RUNTIME, fluentbase_contracts_evm::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, fluentbase_contracts_fairblock::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_IDENTITY, fluentbase_contracts_identity::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, fluentbase_contracts_kzg::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, fluentbase_contracts_multicall::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_NITRO_VERIFIER, fluentbase_contracts_nitro::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, fluentbase_contracts_oauth2::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_RIPEMD160, fluentbase_contracts_ripemd160::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, fluentbase_contracts_ecrecover::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_SHA256, fluentbase_contracts_sha256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, fluentbase_contracts_webauthn::BUILD_OUTPUT),
];

fn devnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: DEVELOPER_PREVIEW_CHAIN_ID,
        homestead_block: Some(0u64),
        dao_fork_block: Some(0u64),
        dao_fork_support: true,
        eip150_block: Some(0u64),
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
        extra_fields: Default::default(),
        parlia: None,
        deposit_contract_address: None,
        prague_time: None,
        osaka_time: None,
        bpo1_time: None,
        bpo2_time: None,
        bpo3_time: None,
        bpo4_time: None,
        blob_schedule: Default::default(),
        bpo5_time: None,
    }
}

fn init_contract(
    alloc: &mut BTreeMap<Address, GenesisAccount>,
    name: &str,
    address: Address,
    rwasm_bytecode: Bytes,
) {
    print!("creating genesis account {} (0x{})... ", name, address);
    std::io::stdout().flush().unwrap();
    println!("{} bytes", rwasm_bytecode.len());
    let mut account = alloc
        .get(&address)
        .cloned()
        .unwrap_or_else(GenesisAccount::default);
    account.code = Some(rwasm_bytecode);
    alloc.insert(address, account);
}

macro_rules! initial_devnet_balance {
    ($address:literal) => {
        (
            address!($address),
            GenesisAccount::default()
                .with_balance(U256::from(1_000_000_000_000_000000000000000000u128)),
        )
    };
}

fn devnet_genesis() -> Genesis {
    let mut alloc = BTreeMap::from([
        // default testing accounts
        initial_devnet_balance!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"), // dmitry
        initial_devnet_balance!("33a831e42B24D19bf57dF73682B9a3780A0435BA"), // daniel
        initial_devnet_balance!("B72988b6DdC94E577E98C5565E0e11E688537e73"), // faucet
        initial_devnet_balance!("c1202e7d42655F23097476f6D48006fE56d38d4f"), // marcus
        initial_devnet_balance!("e92c16763ba7f73a2218a5416aaa493a1f038bef"), // khasan
    ]);

    for (address, contract) in GENESIS_CONTRACTS {
        init_contract(
            &mut alloc,
            contract.name,
            address.clone(),
            contract.rwasm_bytecode.into(),
        );
    }

    Genesis {
        config: devnet_chain_config(),
        nonce: 0,
        timestamp: 0x6490fdd2,
        extra_data: Bytes::new(),
        gas_limit: 0x1c9c380,
        difficulty: U256::ZERO,
        mix_hash: B256::ZERO,
        coinbase: Address::ZERO,
        alloc,
        base_fee_per_gas: None,
        excess_blob_gas: None,
        blob_gas_used: None,
        number: Some(0),
    }
}

fn main() {
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let genesis = devnet_genesis();
    let genesis_json = serde_json::to_string_pretty(&genesis).unwrap();
    let file_name = "genesis-devnet.json";
    let out_dir = cargo_manifest_dir.join(file_name);
    let mut file = File::create(out_dir).unwrap();
    file.write(genesis_json.as_bytes()).unwrap();
    file.sync_all().unwrap();
    file.flush().unwrap();
}

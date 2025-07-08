use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::{
    address,
    compile_wasm_to_rwasm_with_config,
    default_compilation_config,
    keccak256,
    Address,
    Bytes,
    B256,
    DEVELOPER_PREVIEW_CHAIN_ID,
    U256,
};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs,
    io::Write,
    path::PathBuf,
    time::Instant,
};

#[rustfmt::skip]
const GENESIS_CONTRACTS: &[(Address, fluentbase_contracts::BuildOutput)] = &[
    (fluentbase_types::PRECOMPILE_BIG_MODEXP, fluentbase_contracts::FLUENTBASE_CONTRACTS_MODEXP),
    (fluentbase_types::PRECOMPILE_BLAKE2F, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLAKE2F),
    (fluentbase_types::PRECOMPILE_BN256_ADD, fluentbase_contracts::FLUENTBASE_CONTRACTS_BN256),
    (fluentbase_types::PRECOMPILE_BN256_MUL, fluentbase_contracts::FLUENTBASE_CONTRACTS_BN256),
    (fluentbase_types::PRECOMPILE_BN256_PAIR, fluentbase_contracts::FLUENTBASE_CONTRACTS_BN256),
    (fluentbase_types::PRECOMPILE_ERC20_RUNTIME, fluentbase_contracts::FLUENTBASE_CONTRACTS_ERC20),
    (fluentbase_types::PRECOMPILE_EIP2935, fluentbase_contracts::FLUENTBASE_CONTRACTS_EIP2935),
    (fluentbase_types::PRECOMPILE_EVM_RUNTIME, fluentbase_contracts::FLUENTBASE_CONTRACTS_EVM),
    #[cfg(feature="enable-svm")]
    (fluentbase_types::PRECOMPILE_SVM_RUNTIME, fluentbase_contracts::FLUENTBASE_CONTRACTS_SVM),
    (fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, fluentbase_contracts::FLUENTBASE_CONTRACTS_FAIRBLOCK),
    (fluentbase_types::PRECOMPILE_IDENTITY, fluentbase_contracts::FLUENTBASE_CONTRACTS_IDENTITY),
    (fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, fluentbase_contracts::FLUENTBASE_CONTRACTS_KZG),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, fluentbase_contracts::FLUENTBASE_CONTRACTS_BLS12381),
    (fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, fluentbase_contracts::FLUENTBASE_CONTRACTS_MULTICALL),
    (fluentbase_types::PRECOMPILE_NITRO_VERIFIER, fluentbase_contracts::FLUENTBASE_CONTRACTS_NITRO),
    (fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, fluentbase_contracts::FLUENTBASE_CONTRACTS_OAUTH2),
    (fluentbase_types::PRECOMPILE_RIPEMD160, fluentbase_contracts::FLUENTBASE_CONTRACTS_RIPEMD160),
    (fluentbase_types::PRECOMPILE_WASM_RUNTIME, fluentbase_contracts::FLUENTBASE_CONTRACTS_WASM),
    (fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, fluentbase_contracts::FLUENTBASE_CONTRACTS_ECRECOVER),
    (fluentbase_types::PRECOMPILE_SHA256, fluentbase_contracts::FLUENTBASE_CONTRACTS_SHA256),
    (fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, fluentbase_contracts::FLUENTBASE_CONTRACTS_WEBAUTHN),
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

/// Ensures each WASM artifact is compiled to RWASM exactly once, caching the results.
/// This optimization is particularly valuable for large, unoptimized contracts.
fn compile_all_contracts() -> HashMap<&'static [u8], (B256, Bytes)> {
    let mut cache = HashMap::new();

    let config = default_compilation_config().with_builtins_consume_fuel(false);
    for (_, contract) in GENESIS_CONTRACTS {
        if !cache.contains_key(contract.wasm_bytecode) {
            let start = Instant::now();
            let rwasm_bytecode =
                compile_wasm_to_rwasm_with_config(contract.wasm_bytecode, config.clone())
                    .expect("failed to compile wasm to rwasm");
            assert_eq!(rwasm_bytecode.constructor_params.len(), 0);
            let rwasm_bytecode: Bytes = rwasm_bytecode.rwasm_module.serialize().into();
            let hash = keccak256(rwasm_bytecode.as_ref());
            let result = (hash, rwasm_bytecode.clone());
            cache.insert(contract.wasm_bytecode, result.clone());
            println!(
                "{} time={: <3}ms | wasm={: <5}KiB | rwasm={: <5}KiB | increased={:.1}x",
                format!("{: <30}", contract.name), // Pads with dots to 20 chars
                start.elapsed().as_millis(),
                contract.wasm_bytecode.len() / 1024,
                rwasm_bytecode.len() / 1024,
                rwasm_bytecode.len() as f64 / contract.wasm_bytecode.len() as f64,
            );
        }
    }
    cache
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

fn init_contract(
    code: &mut Vec<String>,
    genesis: &mut BTreeMap<Address, GenesisAccount>,
    name: &'static str,
    rwasm_bytecode: Bytes,
    rwasm_bytecode_hash: B256,
    address: Address,
) {
    println!("creating genesis account {} (0x{})... ", name, address);
    let mut account = genesis
        .get(&address)
        .cloned()
        .unwrap_or_else(GenesisAccount::default);
    account.code = Some(rwasm_bytecode.clone());
    genesis.insert(address.clone(), account);

    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join(PathBuf::from(name).with_extension("rwasm"))
        .to_str()
        .unwrap()
        .to_string();
    let rwasm_hash = rwasm_bytecode_hash.to_vec();
    let address = address.to_vec();
    fs::write(&path, rwasm_bytecode.as_ref()).unwrap();
    code.push(format!("\tBuildOutput {{"));
    code.push(format!("\t    name: \"{name}\","));
    code.push(format!("\t    rwasm_bytecode: include_bytes!(\"{path}\"),"));
    code.push(format!("\t    rwasm_bytecode_hash: {rwasm_hash:?},"));
    code.push(format!("\t    address: {address:?}"));
    code.push(format!("\t}},"));
}

fn main() {
    let mut alloc = BTreeMap::from([
        // default testing accounts
        initial_devnet_balance!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"), // dmitry
        initial_devnet_balance!("33a831e42B24D19bf57dF73682B9a3780A0435BA"), // daniel
        initial_devnet_balance!("B72988b6DdC94E577E98C5565E0e11E688537e73"), // faucet
        initial_devnet_balance!("c1202e7d42655F23097476f6D48006fE56d38d4f"), // marcus
        initial_devnet_balance!("e92c16763ba7f73a2218a5416aaa493a1f038bef"), // khasan
    ]);

    let mut code = Vec::new();
    code.push("struct BuildOutput {".to_string());
    code.push("    name: &'static str,".to_string());
    code.push("    rwasm_bytecode: &'static [u8],".to_string());
    code.push("    rwasm_bytecode_hash: [u8; 32],".to_string());
    code.push("    address: [u8; 20],".to_string());
    code.push("}".to_string());
    code.push("static BUILD_OUTPUTS: &[BuildOutput] = &[".to_string());

    let rwasm_artifacts = compile_all_contracts();

    for (address, contract) in GENESIS_CONTRACTS {
        let (rwasm_bytecode_hash, rwasm_bytecode) =
            rwasm_artifacts.get(contract.wasm_bytecode).unwrap().clone();
        init_contract(
            &mut code,
            &mut alloc,
            contract.name,
            rwasm_bytecode,
            rwasm_bytecode_hash,
            address.clone(),
        )
    }

    code.push("];".to_string());
    let code = code.join("\n");

    let genesis = Genesis {
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
    };
    let genesis_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("genesis-devnet.json");
    let genesis = serde_json::to_string_pretty(&genesis).unwrap();
    fs::write(&genesis_path, &genesis).unwrap();

    println!("{}", code);
    let code_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("build_output.rs");
    fs::write(&code_path, &code).unwrap();
}

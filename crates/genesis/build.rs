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
        blob_schedule: Default::default(),
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
            GenesisAccount::default().with_balance(U256::from(1_000_000_000000000000000000u128)),
        )
    };
}

fn devnet_genesis() -> Genesis {
    let mut alloc = BTreeMap::from([
        // default testing accounts
        initial_devnet_balance!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        initial_devnet_balance!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        initial_devnet_balance!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        initial_devnet_balance!("90F79bf6EB2c4f870365E785982E1f101E93b906"),
        initial_devnet_balance!("15d34AAf54267DB7D7c367839AAf71A00a2C6A65"),
        initial_devnet_balance!("9965507D1a55bcC2695C58ba16FB37d819B0A4dc"),
        initial_devnet_balance!("976EA74026E726554dB657fA54763abd0C3a0aa9"),
        initial_devnet_balance!("14dC79964da2C08b23698B3D3cc7Ca32193d9955"),
        initial_devnet_balance!("23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f"),
        initial_devnet_balance!("a0Ee7A142d267C1f36714E4a8F75612F20a79720"),
        initial_devnet_balance!("Bcd4042DE499D14e55001CcbB24a551F3b954096"),
        initial_devnet_balance!("71bE63f3384f5fb98995898A86B02Fb2426c5788"),
        initial_devnet_balance!("FABB0ac9d68B0B445fB7357272Ff202C5651694a"),
        initial_devnet_balance!("1CBd3b2770909D4e10f157cABC84C7264073C9Ec"),
        initial_devnet_balance!("dF3e18d64BC6A983f673Ab319CCaE4f1a57C7097"),
        initial_devnet_balance!("cd3B766CCDd6AE721141F452C550Ca635964ce71"),
        initial_devnet_balance!("2546BcD3c84621e976D8185a91A922aE77ECEc30"),
        initial_devnet_balance!("bDA5747bFD65F08deb54cb465eB87D40e51B197E"),
        initial_devnet_balance!("dD2FD4581271e230360230F9337D5c0430Bf44C0"),
        initial_devnet_balance!("8626f6940E2eb28930eFb4CeF49B2d1F2C9C1199"),
        initial_devnet_balance!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3"),
        initial_devnet_balance!("33a831e42B24D19bf57dF73682B9a3780A0435BA"),
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
    let file_name = "assets/genesis-devnet.json";
    let out_dir = cargo_manifest_dir.join(file_name);
    let mut file = File::create(out_dir).unwrap();
    file.write(genesis_json.as_bytes()).unwrap();
    file.sync_all().unwrap();
    file.flush().unwrap();
}

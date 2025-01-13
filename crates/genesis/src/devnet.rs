use crate::{
    enable_rwasm_contract,
    initial_balance,
    storage_only,
    ChainConfig,
    Genesis,
    GenesisAccount,
    EXAMPLE_FAIRBLOCK_ADDRESS,
    EXAMPLE_GREETING_ADDRESS,
    EXAMPLE_MULTICALL_ADDRESS,
};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{
    address,
    b256,
    Address,
    Bytes,
    B256,
    DEVNET_CHAIN_ID,
    PRECOMPILE_EVM,
    U256,
};
use revm_primitives::keccak256;
use std::collections::BTreeMap;

pub fn devnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: DEVNET_CHAIN_ID,
        homestead_block: Some(0u64),
        dao_fork_block: Some(0u64),
        dao_fork_support: false,
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
    }
}

/// Keccak256("poseidon_hash_key")
pub const GENESIS_POSEIDON_HASH_SLOT: B256 =
    b256!("72adc1368da53d255ed52bce3690fa2b9ec0f64072bcdf3c86adcaf50b54cff1");
/// Keccak256("keccak256_hash_key")
pub const GENESIS_KECCAK_HASH_SLOT: B256 =
    b256!("0215c908b95b16bf09cad5a8f36d2f80c367055b890489abfba6a5f6540b391f");

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev1_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.1.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev4_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.4.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev5_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.5.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis() -> Genesis {
    let mut alloc = BTreeMap::from([
        // default testing accounts
        initial_balance!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        initial_balance!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        initial_balance!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        initial_balance!("90F79bf6EB2c4f870365E785982E1f101E93b906"),
        initial_balance!("15d34AAf54267DB7D7c367839AAf71A00a2C6A65"),
        initial_balance!("9965507D1a55bcC2695C58ba16FB37d819B0A4dc"),
        initial_balance!("976EA74026E726554dB657fA54763abd0C3a0aa9"),
        initial_balance!("14dC79964da2C08b23698B3D3cc7Ca32193d9955"),
        initial_balance!("23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f"),
        initial_balance!("a0Ee7A142d267C1f36714E4a8F75612F20a79720"),
        initial_balance!("Bcd4042DE499D14e55001CcbB24a551F3b954096"),
        initial_balance!("71bE63f3384f5fb98995898A86B02Fb2426c5788"),
        initial_balance!("FABB0ac9d68B0B445fB7357272Ff202C5651694a"),
        initial_balance!("1CBd3b2770909D4e10f157cABC84C7264073C9Ec"),
        initial_balance!("dF3e18d64BC6A983f673Ab319CCaE4f1a57C7097"),
        initial_balance!("cd3B766CCDd6AE721141F452C550Ca635964ce71"),
        initial_balance!("2546BcD3c84621e976D8185a91A922aE77ECEc30"),
        initial_balance!("bDA5747bFD65F08deb54cb465eB87D40e51B197E"),
        initial_balance!("dD2FD4581271e230360230F9337D5c0430Bf44C0"),
        initial_balance!("8626f6940E2eb28930eFb4CeF49B2d1F2C9C1199"),
        storage_only!("ba8ab429ff0aaa5f1bb8f19f1f9974ffc82ff161"),
    ]);

    enable_rwasm_contract!(
        alloc,
        PRECOMPILE_EVM,
        "../../contracts/assets/precompile_evm.wasm"
    );
    // enable_rwasm_contract!(
    //     WCL_CONTRACT_ADDRESS,
    //     "../../contracts/assets/wcl_contract.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_BLAKE2_ADDRESS,
    //     "../../contracts/assets/precompile_blake2.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_BN128_ADDRESS,
    //     "../../contracts/assets/precompile_bn128.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_IDENTITY_ADDRESS,
    //     "../../contracts/assets/precompile_identity.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_KZG_POINT_EVALUATION_ADDRESS,
    //     "../../contracts/assets/precompile_kzg_point_evaluation.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_MODEXP_ADDRESS,
    //     "../../contracts/assets/precompile_modexp.wasm"
    // );
    // enable_rwasm_contract!(
    //     PRECOMPILE_SECP256K1_ADDRESS,
    //     "../../contracts/assets/precompile_secp256k1.wasm"
    // );
    enable_rwasm_contract!(
        alloc,
        EXAMPLE_GREETING_ADDRESS,
        "../../contracts/assets/precompile_greeting.wasm"
    );
    enable_rwasm_contract!(
        alloc,
        EXAMPLE_FAIRBLOCK_ADDRESS,
        "../../contracts/assets/precompile_fairblock.wasm"
    );
    enable_rwasm_contract!(
        alloc,
        EXAMPLE_MULTICALL_ADDRESS,
        "../../contracts/assets/precompile_multicall.wasm"
    );

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

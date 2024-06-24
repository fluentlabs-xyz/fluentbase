use crate::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{address, b256, contracts::PRECOMPILE_EVM, Address, Bytes, B256, U256};
use revm_primitives::keccak256;
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
        extra_fields: Default::default(),
        parlia: None,
        deposit_contract_address: None,
        prague_time: None,
    }
}

/// Keccak256("poseidon_hash_key")
pub const POSEIDON_HASH_KEY: B256 =
    b256!("72adc1368da53d255ed52bce3690fa2b9ec0f64072bcdf3c86adcaf50b54cff1");
/// Keccak256("keccak256_hash_key")
pub const KECCAK_HASH_KEY: B256 =
    b256!("0215c908b95b16bf09cad5a8f36d2f80c367055b890489abfba6a5f6540b391f");

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev1_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.1.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis() -> Genesis {
    macro_rules! initial_balance {
        ($address:literal) => {
            (
                address!($address),
                GenesisAccount {
                    balance: U256::from(100000_000000000000000000u128),
                    ..Default::default()
                },
            )
        };
    }

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
    ]);

    macro_rules! enable_rwasm_contract {
        ($addr:ident, $file_path:literal) => {{
            use std::io::Write;
            let bytecode = Bytes::from(include_bytes!($file_path));
            print!("creating genesis account (0x{})... ", hex::encode($addr));
            std::io::stdout().flush().unwrap();
            let poseidon_hash = poseidon_hash(&bytecode);
            let keccak_hash = keccak256(&bytecode);
            println!("{}", hex::encode(poseidon_hash));
            alloc.insert(
                $addr,
                GenesisAccount {
                    code: Some(bytecode),
                    storage: Some(BTreeMap::from([
                        (POSEIDON_HASH_KEY, poseidon_hash.into()),
                        (KECCAK_HASH_KEY, keccak_hash.into()),
                    ])),
                    ..Default::default()
                },
            );
        }};
    }
    enable_rwasm_contract!(
        PRECOMPILE_EVM,
        "../../contracts/assets/precompile_evm.rwasm"
    );
    // enable_rwasm_contract!(
    //     WCL_CONTRACT_ADDRESS,
    //     "../../contracts/assets/wcl_contract.rwasm"
    // );
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
    // enable_rwasm_contract!(
    //     EXAMPLE_GREETING_ADDRESS,
    //     "../../../examples/greeting/lib.rwasm"
    // );
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

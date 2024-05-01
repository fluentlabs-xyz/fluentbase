use crate::{ChainConfig, Genesis, GenesisAccount, EXAMPLE_GREETING_ADDRESS};
use fluentbase_core::consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{address, b256, Address, Bytes, B256, U256};
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
        // default testing account
        initial_balance!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        // open-zeppelin testing accounts
        initial_balance!("F5F6BC97509fB7F6d5E39d507ACCeECae2abb6f7"),
        initial_balance!("ab6BD638aaB7357cEC168A9d77C51a260A8B6503"),
        initial_balance!("A15125A5a31e6911e31f0C1426BB13eDa676d3a4"),
        initial_balance!("C4dd8B42789b315157C01E2409E086e595Beb489"),
        initial_balance!("Ea3a648B2E00D966797f1f8c117afE9b872Ad058"),
        initial_balance!("C32C445e93679dA7C61e471b2e9dEF2653929D26"),
        initial_balance!("b2CB29E6FcB6F4f398488172CAe9a308A8E68C14"),
        initial_balance!("b34675DC0c51d4b5a1B6e82aaf75D8FdC6fE68C5"),
        initial_balance!("2F5CDAC2ACde59De3E2FDd0F111e91621e0E22a2"),
        initial_balance!("A02dF09Edb72a09f4441328668010fE0f77dA981"),
        initial_balance!("47729Dd7c44d12c74C77425Ff500d8274D9ce227"),
        initial_balance!("D45F610aB00ad979e105cAc69866cFBD3Ac64Cea"),
        initial_balance!("4477F4644C9F72Fd76A5EdB159c9aa2AF9f8885e"),
        initial_balance!("CEf21af3c11501e7Ad6D8386cE1CDe30Bd791de3"),
        initial_balance!("3E3941F848B23e2b24eE5FDaAB11d5D498463Fc8"),
        initial_balance!("d1ce4aB957D293d3eD07aA931df6b1184847F5BC"),
        initial_balance!("41b3F033C659646a87eD1581387f4980AcbFb217"),
        initial_balance!("22aC22A397bCf60E7B6F10c9D6606741A27b4AC0"),
        initial_balance!("F905f7C2B38Af406a55a67979aCFF715c1448FF9"),
        initial_balance!("218aBbc6a1b0F655e9D5c4b7b940cB986B473CE7"),
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

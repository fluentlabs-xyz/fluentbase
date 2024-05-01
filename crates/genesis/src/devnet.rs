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
        initial_balance!("5486aD47fEC44692159173358c78e1Ce0ccc2A6f"),
        initial_balance!("CA9c8FdA293e245F3E5325B6Ef67819A074a604D"),
        initial_balance!("6459beb38129C1c0e270a6e1E9312058611bc8eb"),
        initial_balance!("ba6a3D9c390582045B8d19630a068Df354B4C3E4"),
        initial_balance!("55BB3d5c3F09D93660e67f193ADfA0a4A026FCA7"),
        initial_balance!("1a7c29f5d91fdD2158707679fB0A2cFB8DDC5774"),
        initial_balance!("5aA5947Fd4623b525E1d237675dD7678BaAC3941"),
        initial_balance!("1eBC6B6B1a3231C704B71E02195f5115F5141C3D"),
        initial_balance!("6E6d85C93e584bd8f5449CaB7a7BC5c4c9986541"),
        initial_balance!("419036aabEFBe8d5509EcfbAE3E71C1A61F7DbB4"),
        initial_balance!("9CAe2b78D37595e8716BD3De82D3129D237Ce1Be"),
        initial_balance!("521A0e4932D2CB6a27A26B7E28e7B4679Fec79E2"),
        initial_balance!("56bFC36A3A9B423f33B9aA23890DcA8C9BabF497"),
        initial_balance!("28535269fb136a49dd0b1Fe9d7EAd3E46C9c95CA"),
        initial_balance!("6c068761872871CddaE7abF4eEd1944de86Fc876"),
        initial_balance!("56a886eb7Fb5FE1f2b8C0029c4316E5F2E951849"),
        initial_balance!("2a245DE3f86b5437AA5e7c9DEb8d5Aa6C5388617"),
        initial_balance!("5D8E99a91293B41c7bc19704A303623844028Cee"),
        initial_balance!("98F8aB2754E202C9C022A381E68d512C9E5114eD"),
        initial_balance!("591C2DbCd38Aa30db84CAdB6a1DEB55D7caFC967"),
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

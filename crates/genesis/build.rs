#[cfg(feature = "generate-genesis")]
mod genesis_builder {
    use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
    use cargo_metadata::MetadataCommand;
    use fluentbase_build::cargo_rerun_if_changed;
    use fluentbase_types::{
        address,
        compile_wasm_to_rwasm,
        Address,
        Bytes,
        B256,
        DEVELOPER_PREVIEW_CHAIN_ID,
        PRECOMPILE_BIG_MODEXP,
        PRECOMPILE_BLAKE2F,
        PRECOMPILE_BLS12_381_G1_ADD,
        PRECOMPILE_BLS12_381_G1_MSM,
        PRECOMPILE_BLS12_381_G2_ADD,
        PRECOMPILE_BLS12_381_G2_MSM,
        PRECOMPILE_BLS12_381_MAP_G1,
        PRECOMPILE_BLS12_381_MAP_G2,
        PRECOMPILE_BLS12_381_PAIRING,
        PRECOMPILE_BN256_ADD,
        PRECOMPILE_BN256_MUL,
        PRECOMPILE_BN256_PAIR,
        PRECOMPILE_EVM_RUNTIME,
        PRECOMPILE_FAIRBLOCK_VERIFIER,
        PRECOMPILE_IDENTITY,
        PRECOMPILE_KZG_POINT_EVALUATION,
        PRECOMPILE_NATIVE_MULTICALL,
        PRECOMPILE_RIPEMD160,
        PRECOMPILE_SECP256K1_RECOVER,
        PRECOMPILE_SHA256,
        PRECOMPILE_SVM_RUNTIME,
        U256,
        WASM_SIG,
    };
    use std::{collections::BTreeMap, env, fs::File, io::Write, path::PathBuf};

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
        binary_data: &[u8],
    ) {
        let bytecode: Bytes = if binary_data.starts_with(&WASM_SIG) {
            let result = compile_wasm_to_rwasm(binary_data).unwrap();
            if !result.constructor_params.is_empty() {
                panic!(
                    "rwasm contract ({}) should not have constructor params",
                    name
                );
            }
            result.rwasm_bytecode
        } else {
            Bytes::copy_from_slice(binary_data)
        };
        print!("creating genesis account {} (0x{})... ", name, address);
        std::io::stdout().flush().unwrap();
        println!("{} bytes", bytecode.len());
        let mut account = alloc
            .get(&address)
            .cloned()
            .unwrap_or_else(GenesisAccount::default);
        account.code = Some(bytecode);
        alloc.insert(address, account);
    }

    fn enable_evm_precompiled_contracts(
        alloc: &mut BTreeMap<Address, GenesisAccount>,
        with_bls12: bool,
    ) {
        const WASM_ECRECOVER: &[u8] = include_bytes!("../../contracts/ecrecover/lib.wasm");
        init_contract(
            alloc,
            "secp256k1_recover",
            PRECOMPILE_SECP256K1_RECOVER,
            WASM_ECRECOVER,
        );
        const WASM_SHA256: &[u8] = include_bytes!("../../contracts/sha256/lib.wasm");
        init_contract(alloc, "sha256", PRECOMPILE_SHA256, WASM_SHA256);
        const WASM_RIPEMD160: &[u8] = include_bytes!("../../contracts/ripemd160/lib.wasm");
        init_contract(alloc, "ripemd160", PRECOMPILE_RIPEMD160, WASM_RIPEMD160);
        const WASM_IDENTITY: &[u8] = include_bytes!("../../contracts/identity/lib.wasm");
        init_contract(alloc, "identity", PRECOMPILE_IDENTITY, WASM_IDENTITY);
        const WASM_MODEXP: &[u8] = include_bytes!("../../contracts/modexp/lib.wasm");
        init_contract(alloc, "big_modexp", PRECOMPILE_BIG_MODEXP, WASM_MODEXP);
        const WASM_BN256: &[u8] = include_bytes!("../../contracts/bn256/lib.wasm");
        init_contract(alloc, "bn256_add", PRECOMPILE_BN256_ADD, WASM_BN256);
        init_contract(alloc, "bn256_mul", PRECOMPILE_BN256_MUL, WASM_BN256);
        init_contract(alloc, "bn256_pairing", PRECOMPILE_BN256_PAIR, WASM_BN256);
        const WASM_BLAKE2F: &[u8] = include_bytes!("../../contracts/blake2f/lib.wasm");
        init_contract(alloc, "blake2f", PRECOMPILE_BLAKE2F, WASM_BLAKE2F);
        const WASM_KZG_POINT_EVALUATION: &[u8] = include_bytes!("../../contracts/kzg/lib.wasm");
        init_contract(
            alloc,
            "blake2f",
            PRECOMPILE_KZG_POINT_EVALUATION,
            WASM_KZG_POINT_EVALUATION,
        );
        if with_bls12 {
            const WASM_BLS12381: &[u8] = include_bytes!("../../contracts/bls12381/lib.wasm");
            init_contract(
                alloc,
                "bls12381_g1_add",
                PRECOMPILE_BLS12_381_G1_ADD,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_g1_msm",
                PRECOMPILE_BLS12_381_G1_MSM,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_g2_add",
                PRECOMPILE_BLS12_381_G2_ADD,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_g2_msm",
                PRECOMPILE_BLS12_381_G2_MSM,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_pairing",
                PRECOMPILE_BLS12_381_PAIRING,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_map_g1",
                PRECOMPILE_BLS12_381_MAP_G1,
                WASM_BLS12381,
            );
            init_contract(
                alloc,
                "bls12381_map_g2",
                PRECOMPILE_BLS12_381_MAP_G2,
                WASM_BLS12381,
            );
        }
    }

    #[macro_export]
    macro_rules! initial_devnet_balance {
        ($address:literal) => {
            (
                address!($address),
                GenesisAccount::default()
                    .with_balance(U256::from(1_000_000_000000000000000000u128)),
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

        enable_evm_precompiled_contracts(&mut alloc, false);

        const PRECOMPILE_MULTICALL: &[u8] = include_bytes!("../../contracts/multicall/lib.wasm");
        init_contract(
            &mut alloc,
            "multicall",
            PRECOMPILE_NATIVE_MULTICALL,
            PRECOMPILE_MULTICALL,
        );
        const PRECOMPILE_FAIRBLOCK: &[u8] = include_bytes!("../../contracts/fairblock/lib.wasm");
        init_contract(
            &mut alloc,
            "fairblock",
            PRECOMPILE_FAIRBLOCK_VERIFIER,
            PRECOMPILE_FAIRBLOCK,
        );
        const PRECOMPILE_EVM: &[u8] = include_bytes!("../../contracts/evm/lib.wasm");
        init_contract(&mut alloc, "evm", PRECOMPILE_EVM_RUNTIME, PRECOMPILE_EVM);
        // const PRECOMPILE_SVM: &[u8] = include_bytes!("../../contracts/svm/lib.wasm");
        // init_contract(&mut alloc, "svm", PRECOMPILE_SVM_RUNTIME, PRECOMPILE_SVM);

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

    pub fn generate_genesis() {
        let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let cargo_manifest_path = cargo_manifest_dir.join("Cargo.toml");
        let mut metadata_cmd = MetadataCommand::new();
        let metadata = metadata_cmd
            .manifest_path(cargo_manifest_path)
            .exec()
            .unwrap();
        cargo_rerun_if_changed(&metadata);

        let genesis = devnet_genesis();
        let genesis_json = serde_json::to_string_pretty(&genesis).unwrap();
        let file_name = "assets/genesis-devnet.json";
        let out_dir = cargo_manifest_dir.join(file_name);
        let mut file = File::create(out_dir).unwrap();
        file.write(genesis_json.as_bytes()).unwrap();
        file.sync_all().unwrap();
        file.flush().unwrap();
    }
}

fn main() {
    #[cfg(feature = "generate-genesis")]
    genesis_builder::generate_genesis();
}

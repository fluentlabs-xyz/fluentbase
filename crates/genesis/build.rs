use fluentbase_types::Address;

#[rustfmt::skip]
pub fn get_enabled_system_contracts() -> Vec<(Address, String)> {
    let mut arr = Vec::new();
    arr.extend([
        (fluentbase_types::PRECOMPILE_BIG_MODEXP, "fluentbase-contracts-modexp"),
        (fluentbase_types::PRECOMPILE_BLAKE2F, "fluentbase-contracts-blake2f"),
        (fluentbase_types::PRECOMPILE_BN256_ADD, "fluentbase-contracts-bn256"),
        (fluentbase_types::PRECOMPILE_BN256_MUL, "fluentbase-contracts-bn256"),
        (fluentbase_types::PRECOMPILE_BN256_PAIR, "fluentbase-contracts-bn256"),
        (fluentbase_types::PRECOMPILE_ERC20, "fluentbase-contracts-erc20"),
        (fluentbase_types::PRECOMPILE_EVM_RUNTIME, "fluentbase-contracts-evm"),
        (fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER,"fluentbase-contracts-fairblock",),
        (fluentbase_types::PRECOMPILE_IDENTITY, "fluentbase-contracts-identity"),
        (fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, "fluentbase-contracts-kzg"),
        (fluentbase_types::PRECOMPILE_NATIVE_MULTICALL,"fluentbase-contracts-multicall"),
        (fluentbase_types::PRECOMPILE_NITRO_VERIFIER, "fluentbase-contracts-nitro"),
        (fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, "fluentbase-contracts-oauth2"),
        (fluentbase_types::PRECOMPILE_RIPEMD160, "fluentbase-contracts-ripemd160"),
        (fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, "fluentbase-contracts-ecrecover"),
        (fluentbase_types::PRECOMPILE_SHA256, "fluentbase-contracts-sha256"),
        (fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, "fluentbase-contracts-webauthn"),
    ]);
    #[cfg(feature = "bls12")]
    {
        arr.extend([
            (fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, "fluentbase-contracts-bls12381"),
            (fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, "fluentbase-contracts-bls12381"),
        ]);
    }
    arr.into_iter()
        .map(|(address, name)| (address, name.to_string()))
        .collect()
}

#[cfg(feature = "generate-genesis")]
mod genesis_builder {
    use super::get_enabled_system_contracts;
    use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
    use cargo_metadata::{camino::Utf8PathBuf, MetadataCommand};
    use fluentbase_build::{build_wasm_program, cargo_rerun_if_changed, WasmBuildConfig};
    use fluentbase_types::{
        address,
        compile_wasm_to_rwasm,
        Address,
        Bytes,
        B256,
        DEVELOPER_PREVIEW_CHAIN_ID,
        U256,
        WASM_SIG,
    };
    use std::{collections::BTreeMap, env, fs, fs::File, io::Write, path::PathBuf};

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
        binary_data: Vec<u8>,
    ) {
        let bytecode: Bytes = if binary_data.starts_with(&WASM_SIG) {
            let result = compile_wasm_to_rwasm(&binary_data).unwrap();
            if !result.constructor_params.is_empty() {
                panic!(
                    "rwasm contract ({}) should not have constructor params",
                    name
                );
            }
            result.rwasm_bytecode
        } else {
            Bytes::from(binary_data)
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

        let available_system_contracts: Vec<(String, Utf8PathBuf)> = build_all_system_contracts();
        let enabled_system_contracts: Vec<(Address, String)> = get_enabled_system_contracts();
        let enabled: Vec<(String, Address, Utf8PathBuf)> = enabled_system_contracts
            .into_iter()
            .filter_map(|(address, name)| {
                available_system_contracts
                    .iter()
                    .find(|(available_name, _)| *available_name == name)
                    .map(|(_, path)| (name.clone(), address, path.clone()))
            })
            .collect();

        for (name, address, path) in enabled {
            init_contract(
                &mut alloc,
                &name,
                address,
                fs::read(path).expect("failed to read system precompile"),
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

    pub fn build_precompile_contracts_and_genesis() {
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

    pub fn build_all_system_contracts() -> Vec<(String, Utf8PathBuf)> {
        let mut available_system_contracts = Vec::new();
        fs::read_dir("../../contracts")
            .expect("failed to read directory")
            .for_each(|entry| {
                let path = entry.expect("failed to read entry").path();
                assert!(path.is_dir(), "{} is not a directory", path.display());
                let program = path.to_str().expect("failed to convert path to string");
                let (target_name, wasm_path) = build_wasm_program(
                    WasmBuildConfig::default().with_cargo_manifest_dir(program.to_string()),
                )
                .unwrap();
                println!("compiled system contract {} to {}", target_name, wasm_path);
                available_system_contracts.push((target_name, wasm_path));
            });
        available_system_contracts
    }
}

fn main() {
    #[cfg(feature = "generate-genesis")]
    {
        genesis_builder::build_precompile_contracts_and_genesis();
    }
}

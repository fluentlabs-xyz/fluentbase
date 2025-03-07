#[cfg(feature = "generate-genesis")]
mod genesis_builder {
    use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
    use fluentbase_types::{
        address,
        Address,
        Bytes,
        B256,
        DEVELOPER_PREVIEW_CHAIN_ID,
        EXAMPLE_FAIRBLOCK_ADDRESS,
        EXAMPLE_GREETING_ADDRESS,
        PRECOMPILE_NATIVE_MULTICALL,
        U256,
        WASM_SIG,
    };
    use std::{collections::BTreeMap, env, fs::File, io::Write, path::PathBuf};

    pub fn devnet_chain_config() -> ChainConfig {
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

    #[macro_export]
    macro_rules! initial_balance {
        ($address:literal, $value:expr) => {
            (
                address!($address),
                GenesisAccount::default().with_balance($value),
            )
        };
    }

    fn init_contract(
        alloc: &mut BTreeMap<Address, GenesisAccount>,
        name: &str,
        address: Address,
        binary_data: &[u8],
    ) {
        let bytecode: Bytes = if binary_data.starts_with(&WASM_SIG) {
            let result = fluentbase_types::compile_wasm_to_rwasm(binary_data).unwrap();
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

    pub fn devnet_genesis() -> Genesis {
        let initial_balance = U256::from(1_000_000_000000000000000000u128);
        let mut alloc = BTreeMap::from([
            // default testing accounts
            initial_balance!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266", initial_balance),
            initial_balance!("70997970C51812dc3A010C7d01b50e0d17dc79C8", initial_balance),
            initial_balance!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC", initial_balance),
            initial_balance!("90F79bf6EB2c4f870365E785982E1f101E93b906", initial_balance),
            initial_balance!("15d34AAf54267DB7D7c367839AAf71A00a2C6A65", initial_balance),
            initial_balance!("9965507D1a55bcC2695C58ba16FB37d819B0A4dc", initial_balance),
            initial_balance!("976EA74026E726554dB657fA54763abd0C3a0aa9", initial_balance),
            initial_balance!("14dC79964da2C08b23698B3D3cc7Ca32193d9955", initial_balance),
            initial_balance!("23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f", initial_balance),
            initial_balance!("a0Ee7A142d267C1f36714E4a8F75612F20a79720", initial_balance),
            initial_balance!("Bcd4042DE499D14e55001CcbB24a551F3b954096", initial_balance),
            initial_balance!("71bE63f3384f5fb98995898A86B02Fb2426c5788", initial_balance),
            initial_balance!("FABB0ac9d68B0B445fB7357272Ff202C5651694a", initial_balance),
            initial_balance!("1CBd3b2770909D4e10f157cABC84C7264073C9Ec", initial_balance),
            initial_balance!("dF3e18d64BC6A983f673Ab319CCaE4f1a57C7097", initial_balance),
            initial_balance!("cd3B766CCDd6AE721141F452C550Ca635964ce71", initial_balance),
            initial_balance!("2546BcD3c84621e976D8185a91A922aE77ECEc30", initial_balance),
            initial_balance!("bDA5747bFD65F08deb54cb465eB87D40e51B197E", initial_balance),
            initial_balance!("dD2FD4581271e230360230F9337D5c0430Bf44C0", initial_balance),
            initial_balance!("8626f6940E2eb28930eFb4CeF49B2d1F2C9C1199", initial_balance),
            initial_balance!("390a4CEdBb65be7511D9E1a35b115376F39DbDF3", initial_balance),
            initial_balance!("33a831e42B24D19bf57dF73682B9a3780A0435BA", initial_balance),
            initial_balance!("Ba8AB429Ff0AaA5f1Bb8f19f1f9974fFC82Ff161", U256::ZERO),
        ]);

        const PRECOMPILE_MULTICALL: &[u8] = include_bytes!("../../contracts/multicall/lib.wasm");
        init_contract(
            &mut alloc,
            "multicall",
            PRECOMPILE_NATIVE_MULTICALL,
            PRECOMPILE_MULTICALL,
        );
        const PRECOMPILE_GREETING: &[u8] = include_bytes!("../../examples/greeting/lib.wasm");
        init_contract(
            &mut alloc,
            "greeting",
            EXAMPLE_GREETING_ADDRESS,
            PRECOMPILE_GREETING,
        );
        const PRECOMPILE_FAIRBLOCK: &[u8] = include_bytes!("../../contracts/fairblock/lib.wasm");
        init_contract(
            &mut alloc,
            "fairblock",
            EXAMPLE_FAIRBLOCK_ADDRESS,
            PRECOMPILE_FAIRBLOCK,
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

    pub fn write_genesis_json(genesis: Genesis, file_name: &str) {
        let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let genesis_json = serde_json::to_string_pretty(&genesis).unwrap();
        let out_dir = cargo_manifest_dir.join(file_name);
        println!("cargo:rerun-if-changed={}", out_dir.to_str().unwrap());
        let mut file = File::create(out_dir).unwrap();
        file.write(genesis_json.as_bytes()).unwrap();
        file.sync_all().unwrap();
        file.flush().unwrap();
    }
}

fn main() {
    #[cfg(feature = "generate-genesis")]
    {
        genesis_builder::write_genesis_json(
            genesis_builder::devnet_genesis(),
            "assets/genesis-devnet.json",
        );
    }
}

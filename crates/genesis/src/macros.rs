#[macro_export]
macro_rules! initial_balance {
    ($address:literal, $value:expr) => {
        (
            address!($address),
            GenesisAccount {
                balance: $value,
                ..Default::default()
            },
        )
    };
}

#[macro_export]
macro_rules! enable_rwasm_contract {
    ($alloc:ident, $addr:ident, $file_path:literal) => {{
        use std::io::Write;
        let binary_data = include_bytes!($file_path);
        let bytecode: Bytes = if $file_path.ends_with(".wasm") {
            crate::utils::wasm2rwasm(binary_data)
                .expect("failed to compile WASM to rWASM")
                .into()
        } else {
            binary_data.into()
        };
        print!("creating genesis account (0x{})... ", hex::encode($addr));
        std::io::stdout().flush().unwrap();
        let poseidon_hash = poseidon_hash(&bytecode);
        let keccak_hash = keccak256(&bytecode);
        println!("{} bytes", bytecode.len());
        $alloc.insert(
            $addr,
            GenesisAccount {
                code: Some(bytecode),
                storage: Some(BTreeMap::from([
                    (GENESIS_POSEIDON_HASH_SLOT, poseidon_hash.into()),
                    (GENESIS_KECCAK_HASH_SLOT, keccak_hash.into()),
                ])),
                ..Default::default()
            },
        );
    }};
}

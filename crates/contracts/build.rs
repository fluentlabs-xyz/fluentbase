use fluentbase_types::Address;

#[cfg(feature = "generate-contracts")]
mod genesis_builder {
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
    use std::{
        collections::BTreeMap,
        env,
        fs,
        fs::File,
        io::Write,
        path::PathBuf,
        process::Command,
        str::from_utf8,
    };

    pub fn build_all_system_contracts() /* -> Vec<(String, Utf8PathBuf)> */
    {
        let mut dirs: Vec<String> = Vec::new();
        fs::read_dir("../../contracts")
            .expect("failed to read directory")
            .for_each(|entry| {
                let path = entry.expect("failed to read entry").path();
                assert!(path.is_dir(), "{} is not a directory", path.display());
                let program = path.to_str().expect("failed to convert path to string");
                dirs.push(program.to_string());
            });

        let mut available_system_contracts = Vec::new();
        for dir in dirs {
            // build wasm bytecode for each contract in contracts/**
            let config = WasmBuildConfig::default().with_cargo_manifest_dir(dir);
            let (target_name, wasm_path) = build_wasm_program(config).unwrap();
            println!("compiled system contract {} to {}", target_name, wasm_path);
            available_system_contracts.push((target_name, wasm_path));
        }
        //
        // available_system_contracts
    }
}

fn main() {
    println!("cargo:info=contracts build.rs");
    #[cfg(feature = "generate-contracts")]
    {
        genesis_builder::build_all_system_contracts();
    }
}

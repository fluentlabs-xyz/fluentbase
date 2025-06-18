mod config;

use cargo_metadata::{CrateType, Metadata, MetadataCommand, TargetKind};
pub use config::*;
use fluentbase_types::{compile_wasm_to_rwasm_with_config, default_compilation_config, keccak256};
use rwasm::CompilationConfig;
use std::{env, fs, path::PathBuf, process::Command, str::from_utf8};

pub fn rust_to_wasm(config: RustToWasmConfig) -> PathBuf {
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let cargo_manifest_path = PathBuf::from(cargo_manifest_dir.clone()).join("Cargo.toml");
    let mut metadata_cmd = MetadataCommand::new();
    let metadata = metadata_cmd
        .manifest_path(cargo_manifest_path)
        .exec()
        .unwrap();
    let target_dir: PathBuf = metadata.target_directory.clone().into();
    let target2_dir = target_dir.join("target2");

    let mut args = vec![
        "build".to_string(),
        "--target".to_string(),
        "wasm32-unknown-unknown".to_string(),
        "--release".to_string(),
        "--manifest-path".to_string(),
        format!("{}/Cargo.toml", cargo_manifest_dir.to_str().unwrap()),
        "--target-dir".to_string(),
        target2_dir.to_str().unwrap().to_string(),
        "--color=always".to_string(),
    ];
    if config.no_default_features {
        args.push("--no-default-features".to_string());
    }
    if !config.features.is_empty() {
        args.push("--features".to_string());
        args.extend_from_slice(&config.features);
    }
    let flags = [
        "-C".to_string(),
        format!("link-arg=-zstack-size={}", config.stack_size),
        "-C".to_string(),
        "panic=abort".to_string(),
        "-C".to_string(),
        "target-feature=+bulk-memory".to_string(),
    ];
    let flags = flags.join("\x1f");

    let status = Command::new("cargo")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .args(args)
        .status()
        .expect("WASM compilation failure: failed to run cargo build");

    if !status.success() {
        panic!(
            "WASM compilation failure: failed to run cargo build with code: {}",
            status.code().unwrap_or(1)
        );
    }

    let wasm_artifact_name = calc_wasm_artifact_name(&metadata);
    let wasm_artifact_path = target2_dir
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(wasm_artifact_name);

    wasm_artifact_path
}

pub fn wasm_to_wasmtime(wasm_path: &PathBuf) -> PathBuf {
    let config = wasmtime::Config::new();
    let engine = wasmtime::Engine::new(&config).unwrap();

    let wasm_bytecode = fs::read(&wasm_path).unwrap();
    let module =
        wasmtime::Module::new(&engine, wasm_bytecode).expect("failed to compile wasmtime module");
    let module_bytes = module
        .serialize()
        .expect("failed to serialize wasm bytecode");
    let wasmtime_module_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("lib.cwasm");
    fs::write(&wasmtime_module_path, module_bytes).unwrap();
    wasmtime_module_path
}

pub fn go_to_wasm() -> PathBuf {
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let wasm_artifact_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("lib.wasm");

    let args: Vec<String> = vec![
        "build".to_string(),
        "-o".to_string(),
        wasm_artifact_path.to_str().unwrap().to_string(),
        "--target".to_string(),
        "wasm-unknown".to_string(),
    ];

    let status = Command::new("tinygo")
        .current_dir(&cargo_manifest_dir)
        .args(args)
        .status()
        .expect("WASM compilation failed");

    if !status.success() {
        panic!("WASM compilation failure: failed to run \"tinygo build\"");
    }
    wasm_artifact_path
}

pub fn wasm_to_rwasm(wasm_path: &PathBuf, config: CompilationConfig) -> PathBuf {
    let wasm = fs::read(&wasm_path).unwrap();
    let rwasm: Vec<u8> = compile_wasm_to_rwasm_with_config(wasm.as_slice(), config.clone())
        .unwrap()
        .rwasm_module
        .serialize();
    let rwasm_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("lib.rwasm");
    fs::write(&rwasm_path, &rwasm).unwrap();
    rwasm_path
}

pub fn copy_wasm_and_wat(wasm_path: &PathBuf) {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let wasm_output = dir.join("lib.wasm");
    let wat_output = dir.join("lib.wat");
    fs::copy(&wasm_path, &wasm_output).unwrap();
    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        fs::write(wat_output, from_utf8(&wasm_to_wat.unwrap().stdout).unwrap()).unwrap();
    }
}

fn calc_wasm_artifact_name(metadata: &Metadata) -> String {
    let mut result = vec![];
    for program_crate in metadata.workspace_default_members.to_vec() {
        let program = metadata
            .packages
            .iter()
            .find(|p| p.id == program_crate)
            .unwrap_or_else(|| panic!("cannot find package for {}", program_crate));
        for bin_target in program.targets.iter() {
            let is_bin = bin_target.kind.contains(&TargetKind::Bin)
                && bin_target.crate_types.contains(&CrateType::Bin);
            let is_cdylib = bin_target.kind.contains(&TargetKind::CDyLib)
                && bin_target.crate_types.contains(&CrateType::CDyLib);
            // Both `bin` and `cdylib` crates produce a `.wasm` file
            if is_cdylib || is_bin {
                let bin_name = bin_target.name.clone() + ".wasm";
                result.push(bin_name);
            }
        }
    }
    if result.is_empty() {
        panic!(
            "No WASM artifact found to build in package `{}`. Ensure the package defines exactly one `bin` or `cdylib` crate.",
            metadata.workspace_members.first().unwrap()
        );
    } else if result.len() > 1 {
        panic!(
            "Multiple WASM artifacts found in package `{}`. Ensure the package defines exactly one `bin` or `cdylib` crate.",
            metadata.workspace_members.first().unwrap()
        );
    }
    result.first().unwrap().clone()
}

pub fn is_tinygo_installed() -> bool {
    let output = Command::new("tinygo").arg("-v").output();
    if output.is_err() {
        false
    } else {
        true
    }
}

/// Generates the `build_output.rs` file, which is included in the contract's `lib.rs`.
pub fn generate_build_output_file(
    wasm_path: &PathBuf,
    rwasm_path: &PathBuf,
    wasmtime_path: &PathBuf,
) {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let build_output_path = format!("{}/build_output.rs", out_dir);

    let package_name = env::var("CARGO_PKG_NAME").unwrap();
    let rwasm_hash = keccak256(fs::read(rwasm_path).unwrap());
    let rwasm_hash_hex = rwasm_hash.to_string();
    let rwasm_hash = rwasm_hash.to_vec();

    let wasm_path = wasm_path.to_str().unwrap();
    let rwasm_path = rwasm_path.to_str().unwrap();
    let wasmtime_path = wasmtime_path.to_str().unwrap();
    let code = format!(
        r#"use fluentbase_sdk::GenesisContractBuildOutput;

pub const BUILD_OUTPUT: GenesisContractBuildOutput =
    GenesisContractBuildOutput {{
        name: "{package_name}",
        wasm_bytecode: include_bytes!(r"{wasm_path}"),
        rwasm_bytecode: include_bytes!(r"{rwasm_path}"),
        rwasm_bytecode_hash: {rwasm_hash:?}, // {rwasm_hash_hex}
        wasmtime_module_bytes: include_bytes!(r"{wasmtime_path}"),
    }};
"#
    );

    fs::write(&build_output_path, code).unwrap();
}

pub fn build_default_genesis_contract() {
    build_default_genesis_contract_ext(Default::default())
}

/// Compiles the Genesis contract to WASM, RWASM, and Wasmtime module formats,
/// and generates the corresponding `build_output.rs` file.
/// For non-standard configurations (e.g. custom configurations, source code structure, etc.),
/// it's recommended to copy this function into your own `build.rs` and modify as needed.
pub fn build_default_genesis_contract_ext(rerun_if_changed: &[&str]) {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }
    println!("cargo:rerun-if-changed=src");
    for path in rerun_if_changed {
        println!("cargo:rerun-if-changed={}", path);
    }
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Compile Rust to WASM
    let wasm_path = rust_to_wasm(RustToWasmConfig::default());
    copy_wasm_and_wat(&wasm_path);

    // Compile WASM to RWASM
    let rwasm_config = default_compilation_config().with_builtins_consume_fuel(false);
    let rwasm_path = wasm_to_rwasm(&wasm_path, rwasm_config);

    // Compile WASM to WASMTIME module
    let wasmtime_path = wasm_to_wasmtime(&wasm_path);

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
        wasm_path.to_str().unwrap()
    );

    generate_build_output_file(&wasm_path, &rwasm_path, &wasmtime_path);
}

pub fn build_default_example_contract() {
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return;
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Compile Rust to WASM
    let wasm_path = rust_to_wasm(RustToWasmConfig::default());
    copy_wasm_and_wat(&wasm_path);

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
        wasm_path.to_str().unwrap()
    );
}

mod config;

use cargo_metadata::{camino::Utf8PathBuf, CrateType, Metadata, MetadataCommand, TargetKind};
pub use config::*;
use std::{collections::HashSet, env, fs, path::PathBuf, process::Command, str::from_utf8};

pub fn compile_rust_to_wasm(config: Config) {
    if skip() {
        return;
    }
    let cargo_manifest_dir = PathBuf::from(config.cargo_manifest_dir.clone());
    let cargo_manifest_path = cargo_manifest_dir.join("Cargo.toml");
    let mut metadata_cmd = MetadataCommand::new();
    let metadata = metadata_cmd
        .manifest_path(cargo_manifest_path)
        .exec()
        .unwrap();
    let target_dir = metadata.target_directory.clone();
    let target2_dir = target_dir.join("target2");

    let mut args = vec![
        "build".to_string(),
        "--target".to_string(),
        config.target.clone(),
        "--release".to_string(),
        "--manifest-path".to_string(),
        format!("{}/Cargo.toml", config.cargo_manifest_dir),
        "--target-dir".to_string(),
        target2_dir.to_string(),
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

    let wasm_artifact_name = get_wasm_artifact_name(&metadata);
    let wasm_artifact_path = target2_dir
        .join(config.target.clone())
        .join("release")
        .join(wasm_artifact_name);

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
        wasm_artifact_path
    );

    for path in &config.rerun_if_changed {
        println!("cargo:rerun-if-changed={}", path);
    }

    copy_wasm_to_src(&config, &wasm_artifact_path);
}

pub fn compile_go_to_wasm(config: Config) {
    if skip() {
        return;
    }

    let out_dir = Utf8PathBuf::from(env::var("OUT_DIR").unwrap());
    let wasm_artifact_name = "main.wasm".to_string();
    let wasm_artifact_path = out_dir.join(wasm_artifact_name);

    let args: Vec<String> = vec![
        "build".to_string(),
        "-o".to_string(),
        wasm_artifact_path.to_string(),
        "--target".to_string(),
        "wasm-unknown".to_string(),
    ];

    let status = Command::new("tinygo")
        .current_dir(&config.cargo_manifest_dir)
        .args(args)
        .status()
        .expect("WASM compilation failed");

    if !status.success() {
        panic!("WASM compilation failure: failed to run \"tinygo build\"");
    }

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_ARTIFACT_PATH={}",
        wasm_artifact_path
    );

    for path in &config.rerun_if_changed {
        println!("cargo:rerun-if-changed={}", path);
    }

    copy_wasm_to_src(&config, &wasm_artifact_path);
}

fn copy_wasm_to_src(config: &Config, wasm_artifact_path: &Utf8PathBuf) {
    if config.output_file_name.is_none() {
        return;
    }
    let cargo_manifest_dir = Utf8PathBuf::from(config.cargo_manifest_dir.clone());
    let file_name = config.output_file_name.clone().unwrap();
    let wasm_output = cargo_manifest_dir.join(file_name.clone());
    let wat_output = cargo_manifest_dir.join(file_name.replace(".wasm", ".wat"));

    fs::copy(&wasm_artifact_path, &wasm_output).unwrap();
    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        fs::write(wat_output, from_utf8(&wasm_to_wat.unwrap().stdout).unwrap()).unwrap();
    }
}

fn get_wasm_artifact_name(metadata: &Metadata) -> String {
    let mut result = HashSet::<String>::new();
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
            if is_cdylib || is_bin {
                let bin_name = bin_target.name.clone() + ".wasm";
                result.insert(bin_name);
            }
        }
    }
    if result.len() == 0 {
        panic!(
            "there is no WASM artifact to build for crate {}",
            metadata.workspace_members.first().unwrap()
        );
    } else if result.len() > 1 {
        panic!(
            "multiple WASM artefacts to build for crate {}",
            metadata.workspace_members.first().unwrap()
        );
    }
    result.iter().last().unwrap().clone()
}

fn skip() -> bool {
    if env::var("CARGO_CFG_TARPAULIN").is_ok() {
        return true;
    }
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return true;
    }
    false
}

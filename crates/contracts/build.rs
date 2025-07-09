use cargo_metadata::{CrateType, MetadataCommand, TargetKind};
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let fluentbase_root_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let root_metadata = MetadataCommand::new()
        .manifest_path(&fluentbase_root_dir.join("Cargo.toml"))
        .exec()
        .unwrap();
    let target2_dir: PathBuf = root_metadata.target_directory.join("target2").into();

    let contracts_dir = fluentbase_root_dir.join("contracts");
    println!("cargo:rerun-if-changed={}", contracts_dir.to_str().unwrap());
    let contracts_manifest_path = contracts_dir.join("Cargo.toml");
    // TODO(khasan): why router::test_client_solidity is failing with WASM compiled in debug profile
    // test router::test_client_solidity ... FAILED
    // Caused by:
    //   process didn't exit successfully:
    // `/home/khasan/code/fluentlabs/fluentbase/target/debug/deps/fluentbase_e2e-b00ad5ba2b63abf1`
    // (signal: 11, SIGSEGV: invalid memory reference)
    // let is_debug_profile = env::var("PROFILE").unwrap() == "debug";
    let is_debug_profile = false;
    let metadata = MetadataCommand::new()
        .manifest_path(&contracts_manifest_path)
        .exec()
        .unwrap();

    let mut args = vec![
        "build".to_string(),
        "--target".to_string(),
        "wasm32-unknown-unknown".to_string(),
        "--manifest-path".to_string(),
        contracts_manifest_path.to_str().unwrap().to_string(),
        "--target-dir".to_string(),
        target2_dir.to_str().unwrap().to_string(),
        "--color=always".to_string(),
        "--no-default-features".to_string(),
    ];

    if !is_debug_profile {
        args.push("--release".to_string());
    }

    let flags = vec![
        "-C".to_string(),
        format!("link-arg=-zstack-size={}", 128 * 1024),
        "-C".to_string(),
        "panic=abort".to_string(),
        "-C".to_string(),
        "target-feature=+bulk-memory".to_string(),
    ];
    let encoded_flags = flags.join("\x1f");

    let status = Command::new("cargo")
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_flags)
        .args(args)
        .status()
        .expect("WASM compilation failure: failed to run cargo build");

    if !status.success() {
        panic!(
            "WASM compilation failure: cargo exited with code: {}",
            status.code().unwrap_or(1)
        );
    }

    let artifacts_dir = target2_dir
        .join("wasm32-unknown-unknown")
        .join(if is_debug_profile { "debug" } else { "release" });

    let mut paths: Vec<(String, PathBuf)> = Vec::new();

    for package in &metadata.packages {
        if !metadata.workspace_members.contains(&package.id) {
            continue;
        }
        for target in &package.targets {
            // Check for binary targets
            let is_bin = target.kind.contains(&TargetKind::Bin)
                && target.crate_types.contains(&CrateType::Bin);
            // Check for cdylib targets (also produces wasm binaries)
            let is_cdylib = target.kind.contains(&TargetKind::CDyLib)
                && target.crate_types.contains(&CrateType::CDyLib);

            if is_bin || is_cdylib {
                let mut path = artifacts_dir.clone();
                path.push(&target.name);
                path.set_extension("wasm");
                paths.push((target.name.clone(), path));
            }
        }
    }

    paths.push((
        "fluentbase-contracts-fairblock".to_string(),
        contracts_dir.join("genesis/fairblock/fallback.wasm"),
    ));
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    let mut code = Vec::new();
    code.push("pub struct BuildOutput {".to_string());
    code.push("    pub name: &'static str,".to_string());
    code.push("    pub wasm_bytecode: &'static [u8],".to_string());
    code.push("}".to_string());
    for (name, path) in paths {
        let constant_name = name.to_uppercase().replace('-', "_");
        let path = path.to_str().unwrap();
        code.push(format!(
            "pub const {constant_name}: BuildOutput = BuildOutput {{"
        ));
        code.push(format!("    name: \"{name}\","));
        code.push(format!("    wasm_bytecode: include_bytes!(\"{path}\"),"));
        code.push("};".to_string());
    }
    let code = code.join("\n");
    let build_output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("build_output.rs");

    fs::write(&build_output_path, code).unwrap();
}

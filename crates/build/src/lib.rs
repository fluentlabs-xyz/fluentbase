mod config;

use cargo_metadata::{camino::Utf8PathBuf, CrateType, Metadata, MetadataCommand, TargetKind};
pub use config::*;
use std::{
    env,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    str::from_utf8,
};

fn skip() -> bool {
    if env::var("CARGO_CFG_TARPAULIN").is_ok() {
        return true;
    }
    if env::var("TARGET").unwrap() == "wasm32-unknown-unknown" {
        return true;
    }
    false
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
    // println!("cargo:rerun-if-changed={}", config.cargo_manifest_dir);
    copy_wasm_to_src(&config, &wasm_artifact_path);
}

/// Forces cargo to rerun the build script when any source file in the package or its
/// dependency change.
pub fn cargo_rerun_if_changed(metadata: &Metadata) {
    let root_package = &metadata
        .root_package()
        .expect("should be executed within a Cargo package directory");
    let manifest_path: PathBuf = root_package.manifest_path.clone().into_std_path_buf();
    let package_dir_path: &Path = &manifest_path.parent().unwrap();
    let watch_paths = vec![
        package_dir_path.join("src"),
        package_dir_path.join("bin"),
        package_dir_path.join("Cargo.toml"),
        package_dir_path.join("lib.rs"),
        package_dir_path.join("go.mod"),
        package_dir_path.join("go.sum"),
        package_dir_path.join("main.go"),
    ];
    for path in watch_paths {
        if path.exists() {
            println!(
                "cargo:rerun-if-changed={}",
                path.canonicalize().unwrap().display()
            );
        }
    }
    for dependency in &root_package.dependencies {
        if let Some(path) = &dependency.path {
            println!("cargo:rerun-if-changed={}", path.as_str());
        }
    }
}

fn root_crate_name(metadata: &Metadata) -> String {
    let root_id = metadata
        .resolve
        .as_ref()
        .expect("resolve should be present")
        .root
        .as_ref()
        .expect("root should be present");
    let package = metadata
        .packages
        .iter()
        .find(|p| &p.id == root_id)
        .expect("package should be present");
    package.name.clone()
}

fn get_wasm_artifact_name(metadata: &Metadata) -> String {
    let mut result = vec![];
    for program_crate in metadata.workspace_default_members.to_vec() {
        let program = metadata
            .packages
            .iter()
            .find(|p| p.id == program_crate)
            .unwrap_or_else(|| panic!("cannot find package for {}", program_crate));
        for bin_target in program.targets.iter().filter(|t| {
            t.kind.contains(&TargetKind::Bin) && t.crate_types.contains(&CrateType::Bin)
        }) {
            let bin_name = bin_target.name.clone() + ".wasm";
            result.push(bin_name);
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
    result.first().unwrap().clone()
}

pub fn build_wasm_program(config: Config) -> Option<(String, Utf8PathBuf)> {
    if skip() {
        return None;
    }
    let cargo_manifest_dir = PathBuf::from(config.cargo_manifest_dir.clone());
    let cargo_manifest_path = cargo_manifest_dir.join("Cargo.toml");

    let mut metadata_cmd = MetadataCommand::new();
    let metadata = metadata_cmd
        .manifest_path(cargo_manifest_path)
        .exec()
        .unwrap();

    cargo_rerun_if_changed(&metadata);

    let crate_name = root_crate_name(&metadata);

    let mut artefact_path = compile_rust_to_wasm_2(&config, &metadata);

    if crate_name == "fluentbase-contracts-fairblock" {
        if let Some(path) = compile_go_to_wasm_2(&config) {
            artefact_path = path;
        }
    }

    let wasm_output = cargo_manifest_dir.join(config.output_file_name.clone().unwrap());

    // check that paths are different, or the file will be truncated
    if !wasm_output.canonicalize().is_ok()
        || wasm_output.canonicalize().unwrap() != artefact_path.canonicalize().unwrap()
    {
        fs::copy(&artefact_path, &wasm_output).unwrap();
    }

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_BINARY_PATH_{}={}",
        crate_name, artefact_path
    );

    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        let wast_output = cargo_manifest_dir.join(
            config
                .output_file_name
                .unwrap()
                .clone()
                .replace(".wasm", ".wat"),
        );
        fs::write(
            wast_output,
            from_utf8(&wasm_to_wat.unwrap().stdout).unwrap(),
        )
        .unwrap();
    }
    Some((crate_name, artefact_path))
}

pub fn compile_rust_to_wasm_2(config: &Config, metadata: &Metadata) -> Utf8PathBuf {
    let target_dir = metadata.target_directory.clone();
    let target2_dir = target_dir.join("target2");

    let mut arguments = vec![
        "build".to_string(),
        "--target".to_string(),
        config.target.clone(),
        "--release".to_string(),
        "--manifest-path".to_string(),
        format!("{}/Cargo.toml", config.cargo_manifest_dir),
        "--target-dir".to_string(),
        target2_dir.to_string(),
    ];
    if config.no_default_features {
        arguments.push("--no-default-features".to_string());
    }
    if !config.features.is_empty() {
        arguments.push("--features".to_string());
        arguments.extend_from_slice(&config.features);
    }
    let status = Command::new("cargo")
        .env("CARGO_ENCODED_RUSTFLAGS", get_rust_compiler_flags(&config))
        .args(arguments)
        .status()
        .expect("WASM compilation failure");
    if !status.success() {
        panic!(
            "WASM compilation failure with code: {}",
            status.code().unwrap_or(1)
        );
    }

    let bin_path = target2_dir
        .join(config.target.clone())
        .join("release")
        .join(get_wasm_artifact_name(metadata));
    bin_path
}

pub fn compile_go_to_wasm_2(config: &Config) -> Option<Utf8PathBuf> {
    let cargo_manifest_dir = PathBuf::from(config.cargo_manifest_dir.clone());
    let name = config.output_file_name.clone().unwrap().clone();
    let status = Command::new("tinygo")
        .current_dir(&config.cargo_manifest_dir)
        .args(&["build", "-o", &name, "--target", "wasm-unknown"])
        .status();

    if !status.is_ok() {
        println!("cargo:warning=missing TinyGo, build might be outdated",);
        return None;
    }

    let output_path = match fs::canonicalize(cargo_manifest_dir.join(name)) {
        Ok(absolute_path) => absolute_path,
        Err(e) => panic!("failed to canonicalize path: {}", e),
    };

    Some(Utf8PathBuf::from_path_buf(output_path).unwrap())
}

pub(crate) fn get_rust_compiler_flags(config: &Config) -> String {
    let rust_flags = [
        "-C".to_string(),
        format!("link-arg=-zstack-size={}", config.stack_size),
        "-C".to_string(),
        "panic=abort".to_string(),
        "-C".to_string(),
        "target-feature=+bulk-memory".to_string(),
    ];
    rust_flags.join("\x1f")
}

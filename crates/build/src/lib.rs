mod config;

use cargo_metadata::{camino::Utf8PathBuf, CrateType, Metadata, MetadataCommand, TargetKind};
pub use config::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::from_utf8,
};

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

fn bin_target_name(metadata: &Metadata) -> String {
    let mut result = vec![];
    for program_crate in metadata.workspace_default_members.to_vec() {
        let program = metadata
            .packages
            .iter()
            .find(|p| p.id == program_crate)
            .unwrap_or_else(|| panic!("cannot find package for {}", program_crate));
        for bin_target in program.targets.iter().filter(|t| {
            t.kind.contains(&TargetKind::CDyLib) && t.crate_types.contains(&CrateType::CDyLib)
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

pub fn build_wasm_program(config: WasmBuildConfig) -> Option<(String, Utf8PathBuf)> {
    let cargo_manifest_dir = PathBuf::from(config.cargo_manifest_dir.clone());
    let cargo_manifest_path = cargo_manifest_dir.join("Cargo.toml");

    let mut metadata_cmd = MetadataCommand::new();
    let metadata = metadata_cmd
        .manifest_path(cargo_manifest_path)
        .exec()
        .unwrap();

    cargo_rerun_if_changed(&metadata);

    if config.is_tarpaulin_build {
        println!("cargo:warning=build skipped due to the tarpaulin build");
        return None;
    }

    let crate_name = root_crate_name(&metadata);

    let mut artefact_path = compile_rust_to_wasm(&config, &metadata);

    if crate_name == "fluentbase-contracts-fairblock" {
        if let Some(path) = compile_go_to_wasm(&config) {
            artefact_path = path;
        }
    }

    let wasm_output = cargo_manifest_dir.join(config.output_file_name.clone());

    // check that paths are different, or the file will be truncated
    if !wasm_output.canonicalize().is_ok()
        || wasm_output.canonicalize().unwrap() != artefact_path.canonicalize().unwrap()
    {
        fs::copy(&artefact_path, &wasm_output).unwrap();
    }

    // println!(
    //     "cargo:rustc-env=FLUENTBASE_WASM_BINARY_PATH_{}={}",
    //     crate_name, artefact_path
    // );

    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        let wast_output = cargo_manifest_dir.join(config.output_file_name.replace(".wasm", ".wat"));
        fs::write(
            wast_output,
            from_utf8(&wasm_to_wat.unwrap().stdout).unwrap(),
        )
        .unwrap();
    }
    Some((crate_name, artefact_path))
}

pub fn compile_rust_to_wasm(config: &WasmBuildConfig, metadata: &Metadata) -> Utf8PathBuf {
    let target_dir = metadata.target_directory.clone();
    // let target2_dir = target_dir.join("target2");
    let target2_dir = target_dir;

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
        .join(bin_target_name(metadata));
    bin_path
}

pub fn compile_go_to_wasm(config: &WasmBuildConfig) -> Option<Utf8PathBuf> {
    let cargo_manifest_dir = PathBuf::from(config.cargo_manifest_dir.clone());

    let status = Command::new("tinygo")
        .current_dir(&config.cargo_manifest_dir)
        .args(&[
            "build",
            "-o",
            config.output_file_name.as_str(),
            "--target",
            "wasm-unknown",
        ])
        .status();

    if !status.is_ok() {
        println!("cargo:warning=missing TinyGo, build might be outdated",);
        return None;
    }

    let output_path =
        match fs::canonicalize(cargo_manifest_dir.join(config.output_file_name.clone())) {
            Ok(absolute_path) => absolute_path,
            Err(e) => panic!("failed to canonicalize path: {}", e),
        };

    Some(Utf8PathBuf::from_path_buf(output_path).unwrap())
}

pub(crate) fn get_rust_compiler_flags(config: &WasmBuildConfig) -> String {
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

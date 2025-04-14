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

pub fn calc_wasm_artefact_paths(
    metadata: &Metadata,
    config: &WasmBuildConfig,
) -> (String, Utf8PathBuf) {
    let mut result = vec![];
    let packages_to_iterate = metadata.workspace_default_members.to_vec();
    for program_crate in packages_to_iterate {
        let program = metadata
            .packages
            .iter()
            .find(|p| p.id == program_crate)
            .unwrap_or_else(|| panic!("cannot find package for {}", program_crate));
        for bin_target in program.targets.iter().filter(|t| {
            t.kind.contains(&TargetKind::CDyLib) && t.crate_types.contains(&CrateType::CDyLib)
        }) {
            let bin_name = bin_target.name.clone() + ".wasm";
            let wasm_path = metadata
                .target_directory
                .join(config.target.clone())
                .join("release")
                .join(&bin_name);
            result.push((program.name.clone(), wasm_path));
        }
    }
    if result.is_empty() {
        panic!(
            "there is no WASM artifact to build for crate {}",
            config.cargo_manifest_dir
        );
    } else if result.len() > 1 {
        panic!("multiple WASM artefacts are supported");
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

    if config.current_target.contains("wasm32") {
        println!(
            "cargo:warning=build skipped due to wasm32 compilation target ({})",
            config.current_target
        );
        return None;
    }
    if config.is_tarpaulin_build {
        println!("cargo:warning=build skipped due to the tarpaulin build");
        return None;
    }

    if config.profile == "release" {
        println!("cargo:warning=build skipped due to the release profile");
        return None;
    }

    let (crate_name, mut artefact_path) = calc_wasm_artefact_paths(&metadata, &config);

    compile_rust_to_wasm(&config);

    if crate_name == "fluentbase-contracts-fairblock" {
        if let Some(path) = compile_go_to_wasm(&config) {
            artefact_path = path;
        }
    }

    let wasm_output = cargo_manifest_dir.join(config.output_file_name.clone());

    if !wasm_output.canonicalize().is_ok()
        || wasm_output.canonicalize().unwrap() != artefact_path.canonicalize().unwrap()
    {
        // this condition is needed to prevent file truncation when artefact_path equal to
        // wasm_output
        fs::copy(&artefact_path, &wasm_output).unwrap();
    }

    println!(
        "cargo:rustc-env=FLUENTBASE_WASM_BINARY_PATH_{}={}",
        crate_name, artefact_path
    );

    // Build the project as a WASM binary
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

pub fn compile_rust_to_wasm(config: &WasmBuildConfig) {
    // Build the project as a WASM binary
    let mut arguments = vec![
        "build".to_string(),
        "--target".to_string(),
        config.target.clone(),
        "--release".to_string(),
        "--manifest-path".to_string(),
        format!("{}/Cargo.toml", config.cargo_manifest_dir),
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
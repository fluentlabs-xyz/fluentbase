use cargo_metadata::{camino::Utf8PathBuf, CrateType, Metadata, MetadataCommand, TargetKind};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::from_utf8,
};

const WASM32_TARGET: &str = "wasm32-unknown-unknown";

fn cargo_rerun_if_changed(metadata: &Metadata, program_dir: &Path) {
    let dirs = vec![
        program_dir.join("lib.wasm"),
        program_dir.join("lib.wat"),
        program_dir.join("src"),
        program_dir.join("bin"),
        program_dir.join("build.rs"),
        program_dir.join("Cargo.toml"),
        program_dir.join("lib.rs"),
    ];
    for file in dirs {
        if file.exists() {
            println!(
                "cargo::rerun-if-changed={}",
                file.canonicalize().unwrap().display()
            );
        }
    }
    println!(
        "cargo:rerun-if-changed={}",
        metadata.workspace_root.join("Cargo.lock").as_str()
    );
    for package in &metadata.packages {
        for dependency in &package.dependencies {
            if let Some(path) = &dependency.path {
                println!("cargo:rerun-if-changed={}", path.as_str());
            }
        }
    }
}

pub fn calc_wasm_artefact_paths(metadata: &Metadata) -> Vec<(String, Utf8PathBuf)> {
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
                .join(WASM32_TARGET)
                .join("release")
                .join(&bin_name);
            result.push((bin_name.to_owned(), wasm_path));
        }
    }
    result
}

pub fn build_wasm_program_from_env() {
    // Define output paths
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // let cargo_pkg_name = env::var("CARGO_PKG_NAME").unwrap();
    // let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let cargo_manifest_path = cargo_manifest_dir.join("Cargo.toml");
    let program_dir = Path::new(&cargo_manifest_dir);

    let mut metadata_cmd = MetadataCommand::new();
    let metadata = metadata_cmd
        .manifest_path(cargo_manifest_path)
        .exec()
        .unwrap();
    let root_package = metadata.root_package();
    let root_package_name = root_package
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("program");

    cargo_rerun_if_changed(&metadata, program_dir);

    if target.contains("wasm32") || target.contains("succinct") {
        println!(
            "cargo:warning=build skipped for {} due to wasm32 compilation target ({})",
            root_package_name, target,
        );
        return;
    }

    let artefact_paths = calc_wasm_artefact_paths(&metadata);
    if artefact_paths.is_empty() {
        panic!("there is no WASM artefact to build");
    } else if artefact_paths.len() > 1 {
        panic!("multiple WASM artefacts are supported");
    }

    // try to add wasm32 target
    Command::new("rustup").args(["target", "add", WASM32_TARGET]);

    // Build the project as a WASM binary
    let status = Command::new("cargo")
        .args([
            "build",
            "--target",
            WASM32_TARGET,
            "--release",
            "--no-default-features",
        ])
        .env(
            "RUSTFLAGS",
            "-C link-arg=-zstack-size=262144 -C target-feature=+bulk-memory",
        )
        .status()
        .expect("WASM compilation failure");
    if !status.success() {
        panic!(
            "WASM compilation failure with code: {}",
            status.code().unwrap_or(1)
        );
    }

    let wasm_output = cargo_manifest_dir.join("lib.wasm");
    let wast_output = cargo_manifest_dir.join("lib.wat");

    for (_target_name, wasm_path) in artefact_paths.iter() {
        println!("cargo:rustc-env=FLUENTBASE_WASM_BINARY_PATH={}", wasm_path);
        fs::copy(&wasm_path, &wasm_output).unwrap();
    }

    // Build the project as a WASM binary
    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        fs::write(
            wast_output,
            from_utf8(&wasm_to_wat.unwrap().stdout).unwrap(),
        )
        .unwrap();
    }
}

pub fn build_go_program_from_env() {
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    println!("cargo:rerun-if-changed=go.mod");
    println!("cargo:rerun-if-changed=go.sum");
    println!("cargo:rerun-if-changed=main.go");
    println!("cargo:rerun-if-changed=lib.wasm");

    let is_success = Command::new("tinygo")
        .args(&[
            "build",
            "-o",
            "lib.wasm",
            "--target",
            "wasm-unknown",
            "github.com/fluentlabs-xyz/fluentbase/examples/fairblock",
        ])
        .status()
        .ok()
        .filter(|s| s.success())
        .is_some();
    if !is_success {
        println!("cargo:warning=missing TinyGo, build might be outdated");
    }

    let wasm_output = cargo_manifest_dir.join("lib.wasm");
    let wast_output = cargo_manifest_dir.join("lib.wat");

    // Build the project as a WASM binary
    let wasm_to_wat = Command::new("wasm2wat").args([wasm_output]).output();
    if wasm_to_wat.is_ok() {
        fs::write(
            wast_output,
            from_utf8(&wasm_to_wat.unwrap().stdout).unwrap(),
        )
        .unwrap();
    }
}

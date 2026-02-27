#![allow(clippy::too_many_arguments)]
use crate::{docker, generators, Artifact, BuildArgs, BUILD_TARGET};
use anyhow::{Context, Result};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Result of a successful build
#[derive(Default, Debug)]
pub struct BuildResult {
    pub wasm_path: PathBuf,
    pub wat_path: Option<PathBuf>,
    pub rwasm_path: Option<PathBuf>,
    pub abi_path: Option<PathBuf>,
    pub solidity_path: Option<PathBuf>,
    pub metadata_path: Option<PathBuf>,
    pub foundry_metadata_path: Option<PathBuf>,
}

/// Executes the build process with Docker/local compilation and generates artifacts.
/// Automatically skips execution for cargo check, clippy, and rust-analyzer.
pub(crate) fn build_internal(path: &str, args: Option<BuildArgs>) {
    // Run artifacts generation only for release
    if env::var("PROFILE").unwrap() != "release" {
        return;
    }

    // To avoid recursion we need to check if target is not BUILD_TARGET
    if env::var("TARGET").unwrap() == BUILD_TARGET {
        return;
    }

    // Skip if requested
    if env::var("FLUENTBASE_SKIP_BUILD")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        eprintln!("Build skipped due to FLUENTBASE_SKIP_BUILD");
        return;
    }

    // Skip for clippy and rust-analyzer
    if let Ok(wrapper) = env::var("RUSTC_WORKSPACE_WRAPPER") {
        if wrapper.contains("clippy-driver") || wrapper.contains("rust-analyzer") {
            return;
        }
    }

    // Skip for cargo check
    if env::var("RUSTC").is_err() && env::var("CARGO").is_ok() {
        return;
    }

    let contract_dir = Path::new(path);

    // Canonicalize the path
    let contract_dir = match contract_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => match std::env::current_dir() {
            Ok(cwd) => cwd.join(contract_dir),
            Err(e) => panic!("Failed to determine contract directory: {e}"),
        },
    };

    // Load metadata for rerun-if-changed
    if let Ok(metadata) = MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
    {
        cargo_rerun_if_changed(&metadata, &contract_dir);
    }

    // Execute build
    let args = args.unwrap_or_default();

    match execute_build(&args, Some(contract_dir.to_path_buf())) {
        Ok(result) => {
            println!(
                "cargo:rustc-env=FLUENTBASE_WASM_PATH={}",
                result.wasm_path.display()
            );
            if let Some(path) = &result.rwasm_path {
                println!("cargo:rustc-env=FLUENTBASE_RWASM_PATH={}", path.display());
            }
        }
        Err(err) => panic!("Build failed: {err}"),
    }
}

/// Builds a Rust contract to WASM and optionally generates additional artifacts.
/// Uses Docker for reproducible builds if enabled, otherwise builds locally.
pub fn execute_build(args: &BuildArgs, contract_dir: Option<PathBuf>) -> Result<BuildResult> {
    // Setup
    let contract_dir = contract_dir
        .map(Ok)
        .unwrap_or_else(env::current_dir)
        .context("Failed to determine contract directory")?;

    let rust_toolchain = args.toolchain_version(&contract_dir);

    let metadata = MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
        .context("Failed to load cargo metadata")?;

    let package = metadata
        .root_package()
        .ok_or_else(|| anyhow::anyhow!("No root package found"))?;

    // Determine mount_dir once for all Docker operations
    let mount_dir = args
        .mount_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(|| find_mount_dir(&contract_dir));

    // Determine the Docker image that would be used for all generators
    let docker_image = if args.docker {
        Some(docker::ensure_rust_image(&format!(
            "{}:{}",
            args.docker_image, args.docker_tag
        ))?)
    } else {
        None
    };

    // Build WASM
    let target_wasm_path = build_wasm(args, &contract_dir, package, &docker_image, &mount_dir)?;

    // Early return if no output directory and no artifacts
    if args.output.is_none() && args.generate.is_empty() {
        return Ok(BuildResult {
            wasm_path: target_wasm_path,
            ..Default::default()
        });
    }

    // Check if artifacts requested without output directory
    if args.output.is_none() && !args.generate.is_empty() {
        anyhow::bail!("--output is required when using --generate");
    }

    // From here we know output is Some
    let contract_name = args.contract_name.as_ref().unwrap_or(&package.name);
    let output_dir = args.output.as_ref().unwrap().join(contract_name);

    // Clean the output directory if it exists
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir).with_context(|| {
            format!("Failed to clean output directory: {}", output_dir.display())
        })?;
    }

    // Create fresh output directory
    fs::create_dir_all(&output_dir)?;

    // Copy WASM to output directory
    let output_wasm_path = output_dir.join("lib.wasm");
    fs::copy(&target_wasm_path, &output_wasm_path)?;

    // Initialize result
    let mut result = BuildResult {
        wasm_path: output_wasm_path.clone(),
        ..Default::default()
    };

    // Generate artifacts if requested
    if !args.generate.is_empty() {
        generate_artifacts(
            args,
            &contract_dir,
            &output_dir,
            &output_wasm_path,
            &package.name,
            &docker_image,
            &mount_dir,
            &rust_toolchain,
            &mut result,
        )?;
    }

    Ok(result)
}

fn build_wasm(
    args: &BuildArgs,
    contract_dir: &Path,
    package: &Package,
    docker_image: &Option<String>,
    mount_dir: &Path,
) -> Result<PathBuf> {
    let target_dir = contract_dir
        .join("target")
        .join(crate::HELPER_TARGET_SUBDIR);
    fs::create_dir_all(&target_dir)?;

    // Build cargo command
    let mut cargo_args = args.cargo_build_command();

    if docker_image.is_none() {
        cargo_args.extend(["--target-dir".to_string(), target_dir.display().to_string()]);
    } else {
        cargo_args.extend([
            "--target-dir".to_string(),
            format!("./target/{}", crate::HELPER_TARGET_SUBDIR).to_string(),
        ]);
    }

    // Run build
    let env_vars = vec![("CARGO_ENCODED_RUSTFLAGS".to_string(), args.rust_flags())];

    let docker_config = docker_image
        .as_ref()
        .map(|image| (image.as_str(), mount_dir));

    let rust_toolchain = args.toolchain_version(contract_dir);

    run_command(
        &cargo_args,
        contract_dir,
        docker_config,
        &env_vars,
        &rust_toolchain,
    )?;

    // Find the built WASM file
    let wasm_path = find_wasm_artifact(&target_dir, package)?;

    if args.wasm_opt {
        optimize_wasm(&wasm_path, docker_config, &rust_toolchain)?;
    }

    Ok(wasm_path)
}

fn find_wasm_artifact(target_dir: &Path, package: &Package) -> Result<PathBuf> {
    use cargo_metadata::{CrateType, TargetKind};

    let release_dir = target_dir.join("wasm32-unknown-unknown").join("release");

    // Find all targets that produce WASM
    let wasm_targets: Vec<_> = package
        .targets
        .iter()
        .filter(|target| {
            let is_bin = target.kind.contains(&TargetKind::Bin)
                && target.crate_types.contains(&CrateType::Bin);
            let is_cdylib = target.kind.contains(&TargetKind::CDyLib)
                && target.crate_types.contains(&CrateType::CDyLib);
            is_bin || is_cdylib
        })
        .collect();

    // Validate we have exactly one WASM target
    match wasm_targets.len() {
        0 => anyhow::bail!(
            "No WASM artifact found in package '{}'. \
             Ensure the package defines a `bin` or `cdylib` crate. \
             Add [[bin]] or [lib] with crate-type = [\"cdylib\"] to Cargo.toml",
            package.name
        ),
        1 => {
            let target = wasm_targets[0];
            // Cargo replaces hyphens with underscores in output filenames
            let wasm_filename = format!("{}.wasm", target.name.replace('-', "_"));
            let wasm_path = release_dir.join(&wasm_filename);

            if !wasm_path.exists() {
                anyhow::bail!(
                    "Expected WASM artifact not found: {}\n\
                     Target: {} (kind: {:?}, crate-types: {:?})\n\
                     Make sure the project was built successfully",
                    wasm_path.display(),
                    target.name,
                    target.kind,
                    target.crate_types
                );
            }

            Ok(wasm_path)
        }
        _ => {
            let target_names: Vec<_> = wasm_targets
                .iter()
                .map(|t| format!("{} ({:?})", t.name, t.kind))
                .collect();
            anyhow::bail!(
                "Multiple WASM artifacts found in package '{}': {}\n\
                 Ensure the package defines exactly one `bin` or `cdylib` crate.",
                package.name,
                target_names.join(", ")
            )
        }
    }
}

fn optimize_wasm(
    wasm_path: &Path,
    docker_config: Option<(&str, &Path)>,
    rust_toolchain: &Option<String>,
) -> Result<()> {
    let work_dir = wasm_path.parent().unwrap();
    let wasm_filename = wasm_path.file_name().unwrap().to_str().unwrap();
    let temp_filename = format!("{wasm_filename}.opt");

    run_command(
        &[
            "wasm-opt",
            "-Oz",
            "--enable-bulk-memory",
            "--enable-sign-ext",
            "--enable-mutable-globals",
            wasm_filename,
            "-o",
            &temp_filename,
        ],
        work_dir,
        docker_config,
        &[],
        rust_toolchain,
    )?;

    fs::rename(work_dir.join(&temp_filename), wasm_path)?;
    Ok(())
}

/// Unified function to run commands either in Docker or locally
fn run_command<S: AsRef<str>>(
    args: &[S],
    work_dir: &Path,
    docker_config: Option<(&str, &Path)>,
    env_vars: &[(String, String)],
    rust_toolchain: &Option<String>,
) -> Result<()> {
    let args: Vec<String> = args.iter().map(|s| s.as_ref().to_string()).collect();

    match docker_config {
        Some((image, mount_dir)) => {
            docker::run_in_docker(image, &args, mount_dir, work_dir, env_vars, rust_toolchain)
        }
        None => {
            // Local execution
            let (cmd, rest) = args
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("Empty command"))?;

            let mut command = Command::new(cmd);
            command.args(rest).current_dir(work_dir);

            // Set environment variables
            for (key, value) in env_vars {
                command.env(key, value);
            }

            let status = command.status()?;
            if !status.success() {
                anyhow::bail!("Command failed with exit code: {:?}", status.code());
            }
            Ok(())
        }
    }
}

/// Find the best mount directory for Docker
/// Goes up the directory tree to find a suitable mount point
fn find_mount_dir(work_dir: &Path) -> PathBuf {
    // Try to use the workspace root or current directory
    work_dir
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists() || p.join(".git").exists())
        .unwrap_or(work_dir)
        .to_path_buf()
}

#[allow(clippy::too_many_arguments)]
fn generate_artifacts(
    args: &BuildArgs,
    contract_dir: &Path,
    output_dir: &Path,
    wasm_path: &Path,
    package_name: &str,
    docker_image: &Option<String>,
    mount_dir: &Path,
    rust_toolchain: &Option<String>,
    result: &mut BuildResult,
) -> Result<()> {
    // Helper to write JSON files
    fn write_json<T: serde::Serialize>(path: &Path, data: &T) -> Result<()> {
        let json = serde_json::to_string_pretty(data)?;
        fs::write(path, json)?;
        Ok(())
    }

    // Pre-generate ABI if needed by any artifact
    let needs_abi = args
        .generate
        .iter()
        .any(|a| matches!(a, Artifact::Abi | Artifact::Solidity | Artifact::Metadata));

    let abi = if needs_abi {
        Some(generators::solidity::generate_abi(contract_dir)?)
    } else {
        None
    };

    // Generate artifacts in dependency order
    let mut artifacts = args.generate.clone();
    artifacts.sort_by_key(|a| {
        match a {
            Artifact::Rwasm => 0,    // Can be generated independently
            Artifact::Wat => 1,      // Can be generated independently
            Artifact::Abi => 2,      // Depends on pre-generated ABI
            Artifact::Solidity => 3, // Depends on ABI
            Artifact::Metadata => 4, // Depends on everything else
            Artifact::Foundry => 5,  // Depends on everything else
        }
    });

    // Cache wasm data to avoid re-reading
    let mut wasm_data: Option<Vec<u8>> = None;

    for artifact in &artifacts {
        match artifact {
            Artifact::Wat => {
                let docker_config = docker_image
                    .as_ref()
                    .map(|image| (image.as_str(), mount_dir));

                run_command(
                    &["wasm2wat", "lib.wasm", "-o", "lib.wat"],
                    output_dir,
                    docker_config,
                    &[],
                    rust_toolchain,
                )?;
                result.wat_path = Some(output_dir.join("lib.wat"));
            }
            // We don't need to run wasm -> rwasm inside docker since
            // it's a pure Rust library function without platform dependencies
            Artifact::Rwasm => {
                let data = wasm_data
                    .get_or_insert_with(|| fs::read(wasm_path).expect("Failed to read WASM file"));

                let rwasm_result = fluentbase_sdk::compile_wasm_to_rwasm(data)
                    .map_err(|e| anyhow::anyhow!("rWASM compilation failed: {:?}", e))?;

                let rwasm_path = output_dir.join("lib.rwasm");
                fs::write(&rwasm_path, rwasm_result.rwasm_module.serialize())?;
                result.rwasm_path = Some(rwasm_path);
            }

            Artifact::Abi => {
                let abi = abi.as_ref().expect("ABI should be pre-generated");
                let abi_path = output_dir.join("abi.json");
                write_json(&abi_path, abi)?;
                result.abi_path = Some(abi_path);
            }

            Artifact::Solidity => {
                let abi = abi.as_ref().expect("ABI should be pre-generated");
                let interface = generators::solidity::generate_interface(package_name, abi)?;
                let sol_path = output_dir.join("interface.sol");
                fs::write(&sol_path, interface)?;
                result.solidity_path = Some(sol_path);
            }

            Artifact::Metadata => {
                let wasm = wasm_data
                    .get_or_insert_with(|| fs::read(wasm_path).expect("Failed to read WASM file"));

                let rwasm_data = result.rwasm_path.as_ref().map(fs::read).transpose()?;

                let metadata = generators::metadata::generate(
                    contract_dir,
                    args,
                    wasm,
                    rwasm_data.as_deref(),
                    docker_image.as_deref(),
                    rust_toolchain.as_deref(),
                )?;

                let meta_path = output_dir.join("metadata.json");
                write_json(&meta_path, &metadata)?;
                result.metadata_path = Some(meta_path);
            }

            Artifact::Foundry => {
                let abi = abi.as_ref().expect("ABI should be pre-generated");
                let wasm = wasm_data
                    .get_or_insert_with(|| fs::read(wasm_path).expect("Failed to read WASM file"));

                let rwasm_data_path = result
                    .rwasm_path
                    .as_ref()
                    .expect("rwasm is required for Foundry artifact - generate rwasm first");

                let rwasm_data =
                    fs::read(rwasm_data_path).expect("rwasm should be generated at this point");

                // Create minimal build metadata for Foundry artifact
                // (if full metadata is needed, we'd need to generate it first)
                let build_metadata = generators::metadata::generate(
                    contract_dir,
                    args,
                    wasm,
                    Some(&rwasm_data),
                    docker_image.as_deref(),
                    rust_toolchain.as_deref(),
                )?;

                let interface_path = format!("{package_name}.wasm/interface.sol");
                let foundry_artifact = generators::foundry::generate_artifact(
                    package_name,
                    &serde_json::to_value(abi)?,
                    wasm,
                    &rwasm_data,
                    &build_metadata,
                    &interface_path,
                )?;

                let foundry_path = output_dir.join("foundry.json");
                write_json(&foundry_path, &foundry_artifact)?;
                result.foundry_metadata_path = Some(foundry_path);
            }
        }
    }

    Ok(())
}

fn cargo_rerun_if_changed(metadata: &Metadata, contract_dir: &Path) {
    // Watch source files
    for path in &["src", "Cargo.toml", "build.rs"] {
        let full_path = contract_dir.join(path);
        if full_path.exists() {
            println!("cargo:rerun-if-changed={}", full_path.display());
        }
    }

    // Watch Cargo.lock
    let lock_path = metadata.workspace_root.join("Cargo.lock");
    if lock_path.exists() {
        println!("cargo:rerun-if-changed={lock_path}");
    }
}

use crate::{
    command::{self, DockerConfig},
    generators,
    Artifact,
    BuildArgs,
    BUILD_TARGET,
};
use anyhow::{Context, Result};
use cargo_metadata::{Metadata, MetadataCommand};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
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
}

pub fn execute_build(args: &BuildArgs, contract_dir: Option<PathBuf>) -> Result<BuildResult> {
    // Setup
    let contract_dir = contract_dir
        .map(Ok)
        .unwrap_or_else(env::current_dir)
        .context("Failed to determine contract directory")?;

    let metadata = MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
        .context("Failed to load cargo metadata")?;

    let package = metadata
        .root_package()
        .ok_or_else(|| anyhow::anyhow!("No root package found"))?;

    // Build WASM in target directory
    let target_wasm_path = build_wasm(args, &contract_dir, &package.name)?;

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
    let output_dir = args.output.as_ref().unwrap().join(&package.name);
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
            &mut result,
        )?;
    }

    Ok(result)
}

pub(crate) fn build_internal(path: &str, args: Option<BuildArgs>) {
    if env::var("TARGET").unwrap() == BUILD_TARGET {
        return;
    }
    let contract_dir = Path::new(path);

    // Canonicalize the path to avoid relative path issues
    let contract_dir = match contract_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            // If canonicalize fails, try to get absolute path
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(contract_dir),
                Err(e) => panic!("Failed to determine contract directory: {}", e),
            }
        }
    };

    // Skip if requested
    if env::var("FLUENTBASE_SKIP_BUILD")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        eprintln!("Build skipped due to FLUENTBASE_SKIP_BUILD");
        return;
    }

    // Skip for clippy
    if env::var("RUSTC_WORKSPACE_WRAPPER")
        .map(|val| val.contains("clippy-driver"))
        .unwrap_or(false)
    {
        return;
    }

    // Load metadata for rerun-if-changed
    if let Ok(metadata) = MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
    {
        cargo_rerun_if_changed(&metadata, &contract_dir);
    }

    // Execute build
    let mut args = args.unwrap_or_default();

    args.mount_dir = Some(get_mount_dir(args.mount_dir.take(), &contract_dir));

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

fn get_mount_dir(specified: Option<PathBuf>, fallback: &Path) -> PathBuf {
    specified
        .or_else(|| std::env::current_dir().ok())
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_else(|| fallback.to_path_buf())
}

fn build_wasm(args: &BuildArgs, contract_dir: &Path, package_name: &str) -> Result<PathBuf> {
    // Determine target directory
    let target_subdir = if args.docker { "docker" } else { "local" };
    let target_dir = contract_dir
        .join("target")
        .join(crate::HELPER_TARGET_SUBDIR)
        .join(target_subdir);
    fs::create_dir_all(&target_dir)?;

    // Build cargo command
    let cargo_args = args.cargo_build_command();

    // Build
    if args.docker {
        command::run(
            &cargo_args,
            contract_dir,
            Some(DockerConfig {
                sdk_tag: args.tag.clone(),
                rust_version: args.get_rust_version(contract_dir),
                env_vars: vec![("CARGO_ENCODED_RUSTFLAGS".to_string(), args.rustflags())],
                mount_dir: args.mount_dir.clone().unwrap(),
            }),
        )?;
    } else {
        // Local build
        let mut local_args = cargo_args;
        local_args.extend(["--target-dir".to_string(), target_dir.display().to_string()]);
        env::set_var("CARGO_ENCODED_RUSTFLAGS", args.rustflags());
        command::run(&local_args, contract_dir, None)?;
    }

    let wasm_filename = format!("{}.wasm", package_name.replace('-', "_"));
    let wasm_path = target_dir
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(&wasm_filename);

    if !wasm_path.exists() {
        // Try alternative names
        let alt_paths = [
            target_dir
                .join("wasm32-unknown-unknown")
                .join("release")
                .join(format!("{}.wasm", package_name)),
            target_dir
                .join("wasm32-unknown-unknown")
                .join("release")
                .join("main.wasm"),
        ];

        for alt in &alt_paths {
            if alt.exists() {
                if args.wasm_opt {
                    optimize_wasm(alt, args)?;
                }
                return Ok(alt.clone());
            }
        }

        anyhow::bail!("WASM artifact not found. Expected: {}", wasm_path.display());
    }

    // Optimize in place if requested
    if args.wasm_opt {
        optimize_wasm(&wasm_path, args)?;
    }

    Ok(wasm_path)
}

fn generate_artifacts(
    args: &BuildArgs,
    contract_dir: &Path,
    output_dir: &Path,
    wasm_path: &Path,
    package_name: &str,
    result: &mut BuildResult,
) -> Result<()> {
    // Pre-generate ABI if needed
    let abi = if args
        .generate
        .iter()
        .any(|a| matches!(a, Artifact::Abi | Artifact::Solidity | Artifact::Metadata))
    {
        Some(generators::solidity::generate_abi(contract_dir)?)
    } else {
        None
    };

    // Generate each artifact
    for artifact in &args.generate {
        match artifact {
            Artifact::Wat => {
                run_tool(&["wasm2wat", "lib.wasm", "-o", "lib.wat"], output_dir, args)?;
                result.wat_path = Some(output_dir.join("lib.wat"));
            }
            Artifact::Rwasm => {
                let wasm_data = fs::read(wasm_path)?;
                let rwasm_result = fluentbase_types::compile_wasm_to_rwasm(&wasm_data)
                    .map_err(|e| anyhow::anyhow!("rWASM compilation failed: {:?}", e))?;
                let rwasm_path = output_dir.join("lib.rwasm");
                fs::write(&rwasm_path, rwasm_result.rwasm_bytecode.to_vec())?;
                result.rwasm_path = Some(rwasm_path);
            }
            Artifact::Abi => {
                let abi = abi.as_ref().expect("ABI should be pre-generated");
                let abi_path = output_dir.join("abi.json");
                fs::write(&abi_path, serde_json::to_string_pretty(abi)?)?;
                result.abi_path = Some(abi_path);
            }
            Artifact::Solidity => {
                let abi = abi.as_ref().expect("ABI should be pre-generated");
                let interface = generators::solidity::generate_interface(&package_name, abi)?;
                let sol_path = output_dir.join(format!("I{}.sol", package_name));
                fs::write(&sol_path, interface)?;
                result.solidity_path = Some(sol_path);
            }
            Artifact::Metadata => {
                let wasm_data = fs::read(wasm_path)?;
                let rwasm_data = result.rwasm_path.as_ref().map(fs::read).transpose()?;
                let metadata = generators::metadata::generate(
                    contract_dir,
                    args,
                    &wasm_data,
                    rwasm_data.as_deref(),
                    abi.as_ref(),
                )?;
                let meta_path = output_dir.join("metadata.json");
                fs::write(&meta_path, serde_json::to_string_pretty(&metadata)?)?;
                result.metadata_path = Some(meta_path);
            }
        }
    }

    Ok(())
}

fn optimize_wasm(wasm_path: &Path, args: &BuildArgs) -> Result<()> {
    let temp_path = wasm_path.with_extension("opt.wasm");

    // Run wasm-opt
    run_tool(
        &[
            "wasm-opt",
            "-Oz",                      // Optimize for size
            "--enable-bulk-memory",     // Enable bulk memory operations
            "--enable-sign-ext",        // Enable sign extension operations
            "--enable-mutable-globals", // Enable mutable globals
            wasm_path.to_str().unwrap(),
            "-o",
            temp_path.to_str().unwrap(),
        ],
        wasm_path.parent().unwrap(),
        args,
    )?;

    // Replace original
    fs::rename(&temp_path, wasm_path)?;
    Ok(())
}

fn run_tool(cmd: &[&str], work_dir: &Path, args: &BuildArgs) -> Result<()> {
    let cmd_vec: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();

    if args.docker {
        command::run(
            &cmd_vec,
            work_dir,
            Some(DockerConfig {
                sdk_tag: args.tag.clone(),
                rust_version: None,
                env_vars: vec![],
                mount_dir: args.mount_dir.clone().unwrap(),
            }),
        )
    } else {
        command::run(&cmd_vec, work_dir, None)
    }
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
        println!("cargo:rerun-if-changed={}", lock_path);
    }
}

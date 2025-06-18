mod build;
mod docker;
mod generators;
mod internal;
mod utils;

use crate::build::build_internal;
pub use build::{execute_build, BuildResult};
use clap::{Parser, ValueEnum};
pub use internal::*;
use std::{
    env,
    path::{Path, PathBuf},
};

// Build configuration constants
pub const DEFAULT_DOCKER_TAG: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const DEFAULT_STACK_SIZE: u32 = 128 * 1024;
pub const BUILD_TARGET: &str = "wasm32-unknown-unknown";
pub const HELPER_TARGET_SUBDIR: &str = "wasm-compilation";

/// Build contract at specified path
///
/// Set `FLUENTBASE_SKIP_BUILD` environment variable to skip building.
pub fn build(path: &str) {
    build_internal(path, None)
}

/// Build contract with custom configuration
///
/// Set `FLUENTBASE_SKIP_BUILD` environment variable to skip building.
pub fn build_with_args(path: &str, args: BuildArgs) {
    build_internal(path, Some(args))
}

/// Types of artifacts that can be generated
#[derive(Clone, ValueEnum, Debug, PartialEq)]
pub enum Artifact {
    /// WebAssembly Text format
    Wat,
    /// Reduced WebAssembly
    Rwasm,
    /// Solidity ABI JSON
    Abi,
    /// Solidity interface file
    Solidity,
    /// Contract verification metadata
    Metadata,
}

/// Build configuration for Fluent smart contracts
#[derive(Clone, Parser, Debug)]
pub struct BuildArgs {
    /// Custom contract name for output directory (defaults to package name)
    #[arg(long)]
    pub contract_name: Option<String>,

    /// Run compilation in Docker for reproducible builds
    #[arg(long)]
    pub docker: bool,

    /// Docker image tag to use
    #[arg(long, default_value = DEFAULT_DOCKER_TAG)]
    pub tag: String,

    /// Root directory to mount in Docker (defaults to current directory)
    #[arg(long)]
    pub mount_dir: Option<PathBuf>,

    /// Rust toolchain version (e.g., "1.85.0", "nightly-2024-01-01")
    /// If not specified, will check rust-toolchain.toml, then use base image version
    #[arg(long)]
    pub rust_version: Option<String>,

    /// Cargo features to activate (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,

    /// Do not activate default features
    #[arg(long)]
    pub no_default_features: bool,

    /// Ensure Cargo.lock remains unchanged
    #[arg(long)]
    pub locked: bool,

    /// Stack size for WASM module in bytes
    #[arg(long, default_value_t = DEFAULT_STACK_SIZE)]
    pub stack_size: u32,

    /// Extra rustc flags (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub rustflags: Vec<String>,

    /// Additional artifacts to generate
    #[arg(short = 'g', long, value_enum, value_delimiter = ',')]
    pub generate: Vec<Artifact>,

    /// Output directory for artifacts
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Post process wasm for size optimization
    #[arg(long)]
    pub wasm_opt: bool,
}

impl Default for BuildArgs {
    fn default() -> Self {
        Self {
            contract_name: None,
            docker: true,
            tag: DEFAULT_DOCKER_TAG.to_string(),
            mount_dir: None,
            rust_version: None,
            features: vec![],
            no_default_features: true,
            locked: true,
            stack_size: DEFAULT_STACK_SIZE,
            rustflags: vec![],
            generate: vec![],
            output: Some(PathBuf::from("./out")),
            wasm_opt: true,
        }
    }
}

impl BuildArgs {
    /// Get Rust version to use, checking multiple sources
    pub fn toolchain_version(&self, contract_dir: &Path) -> Option<String> {
        // 1. CLI argument takes precedence
        if let Some(ref version) = self.rust_version {
            return Some(version.clone());
        }

        // 2. Check rust-toolchain.toml
        let toolchain_toml = contract_dir.join("rust-toolchain.toml");
        if toolchain_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&toolchain_toml) {
                // Simple parsing for channel
                // Format: [toolchain]\nchannel = "1.85.0"
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with("channel") && line.contains('=') {
                        if let Some(version) = line.split('=').nth(1) {
                            let version = version.trim().trim_matches('"').trim_matches('\'');
                            return Some(version.to_string());
                        }
                    }
                }
            }
        }

        // 3. Check legacy rust-toolchain file
        let toolchain_file = contract_dir.join("rust-toolchain");
        if toolchain_file.exists() {
            if let Ok(version) = std::fs::read_to_string(&toolchain_file) {
                let trimmed = version.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }

        // 4. None - will use base image version
        None
    }

    /// Generate cargo build command
    pub fn cargo_build_command(&self) -> Vec<String> {
        let mut cmd = vec![
            "cargo".to_string(),
            "build".to_string(),
            "--target".to_string(),
            BUILD_TARGET.to_string(),
            "--release".to_string(),
        ];

        if self.docker {
            let target_subdir = "docker";
            cmd.push("--target-dir".to_string());
            cmd.push(format!("target/{}/{}", HELPER_TARGET_SUBDIR, target_subdir));
        }

        if self.no_default_features {
            cmd.push("--no-default-features".to_string());
        }

        if !self.features.is_empty() {
            cmd.push("--features".to_string());
            cmd.push(self.features.join(","));
        }

        if self.locked {
            cmd.push("--locked".to_string());
        }

        cmd.push("--color=always".to_string());
        cmd
    }

    /// Generate RUSTFLAGS for WASM compilation
    pub fn rustflags(&self) -> String {
        let mut flags = vec![
            format!("-Clink-arg=-zstack-size={}", self.stack_size),
            "-Cpanic=abort".to_string(),
            "-Ctarget-feature=+bulk-memory".to_string(),
        ];

        if self.wasm_opt {
            flags.push("-Copt-level=z".to_string());
            // TODO: should we actually use this optimizations?
            flags.push("-Clto=fat".to_string());
            flags.push("-Cstrip=symbols".to_string());
        }

        // Custom flags
        for flag in &self.rustflags {
            if flag.contains("panic=") && !flag.contains("panic=abort") {
                eprintln!("Warning: Overriding panic=abort may cause issues with WASM");
            }
            if flag.contains("link-arg=-zstack-size") {
                eprintln!("Warning: Stack size flag may override configured value");
            }
            flags.push(flag.to_string());
        }

        flags.join("\x1f")
    }
}

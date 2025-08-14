mod build;
mod docker;
mod generators;
mod internal;
mod utils;

use crate::build::build_internal;
pub use build::{execute_build, BuildResult};
pub use generators::*;
use clap::{Parser, ValueEnum};
pub use internal::*;
use std::{
    env,
    path::{Path, PathBuf},
};

// Build configuration constants
pub const DEFAULT_DOCKER_IMAGE: &str = "ghcr.io/fluentlabs-xyz/fluentbase-build";
pub const DEFAULT_DOCKER_TAG: &str = concat!("v", env!("CARGO_PKG_VERSION"));
pub const DOCKER_PLATFORM: &str = "linux/amd64";

pub const DEFAULT_STACK_SIZE: u32 = 128 * 1024; // 128 KB
pub const BUILD_TARGET: &str = "wasm32-unknown-unknown";
pub const HELPER_TARGET_SUBDIR: &str = "wasm-compilation";
pub const DEFAULT_RUST_TOOLCHAIN: &str = "1.88";

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
    /// Foundry metadata,
    Foundry,
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

    #[arg(long, default_value = DEFAULT_DOCKER_IMAGE)]
    pub docker_image: String,

    /// Docker image tag to use
    #[arg(long, default_value = DEFAULT_DOCKER_TAG)]
    pub docker_tag: String,

    /// Root directory to mount in Docker (defaults to current directory)
    #[arg(long)]
    pub mount_dir: Option<PathBuf>,

    /// Rust toolchain version (e.g., "1.85.0", "nightly-2024-01-01")
    /// If not specified, will check rust-toolchain.toml, then use base image version
    #[arg(long)]
    pub rust_version: Option<String>,

    /// Explicitly use the rust-toolchain.toml file from the contract's directory.
    #[arg(long)]
    pub use_toolchain_file: bool,

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
            docker_image: DEFAULT_DOCKER_IMAGE.to_string(),
            docker_tag: DEFAULT_DOCKER_TAG.to_string(),
            mount_dir: None,
            rust_version: None,
            use_toolchain_file: false, // by default use rust from the image
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
    pub fn toolchain_version(&self, contract_dir: &Path) -> Option<String> {
        if let Some(version) = &self.rust_version {
            return Some(version.clone());
        }
        if self.use_toolchain_file {
            return Self::find_toolchain_in_files(contract_dir);
        }
        Some(DEFAULT_RUST_TOOLCHAIN.to_string())
    }
    /// Finds and parses the toolchain version by checking `rust-toolchain.toml`
    /// and then the legacy `rust-toolchain` file.
    ///
    /// This function defines the priority for discovering the toolchain version from files:
    /// 1. It first looks for `rust-toolchain.toml` and parses the `channel` value.
    /// 2. If not found, it falls back to reading the legacy `rust-toolchain` file.
    ///
    /// Returns `None` if no valid toolchain file is found.
    pub fn find_toolchain_in_files(contract_dir: &Path) -> Option<String> {
        Self::parse_toml_toolchain(contract_dir)
            .or_else(|| Self::parse_legacy_toolchain(contract_dir))
    }

    /// Parses the `channel` from a `rust-toolchain.toml` file.
    fn parse_toml_toolchain(contract_dir: &Path) -> Option<String> {
        // Read the file content. The `?` operator will return `None` if reading fails.
        let content = std::fs::read_to_string(contract_dir.join("rust-toolchain.toml")).ok()?;

        // Use `find_map` to iterate through lines and stop at the first successful parse.
        // This is much cleaner than nested loops and ifs.
        content.lines().find_map(|line| {
            // A more direct way to extract the value using `strip_prefix`.
            // The chain of `?` makes this very robust against malformed lines.
            let value_part = line
                .trim()
                .strip_prefix("channel")?
                .trim()
                .strip_prefix('=')?
                .trim();

            // Remove quotes from the version string.
            let version = value_part.trim_matches(|c| c == '"' || c == '\'');

            // Normalize and return if valid.
            Self::normalize_toolchain(version)
        })
    }

    /// Reads the toolchain version from a legacy `rust-toolchain` file.
    fn parse_legacy_toolchain(contract_dir: &Path) -> Option<String> {
        // Read the file. The `?` handles the "file not found" case.
        let version = std::fs::read_to_string(contract_dir.join("rust-toolchain")).ok()?;
        let trimmed = version.trim();

        // Ensure the file is not empty before normalizing.
        if trimmed.is_empty() {
            None
        } else {
            Self::normalize_toolchain(trimmed)
        }
    }

    /// Normalize toolchain by removing architecture suffix
    fn normalize_toolchain(toolchain: &str) -> Option<String> {
        // Remove any architecture/platform suffix
        let normalized = toolchain
            .split('-')
            .take_while(|part| {
                !matches!(
                    *part,
                    "x86_64"
                        | "aarch64"
                        | "i686"
                        | "arm"
                        | "windows"
                        | "linux"
                        | "darwin"
                        | "apple"
                        | "pc"
                        | "unknown"
                        | "gnu"
                        | "msvc"
                )
            })
            .collect::<Vec<_>>()
            .join("-");

        // Don't allow generic channels
        if matches!(normalized.as_str(), "stable" | "nightly" | "beta") {
            eprintln!("Error: Generic channel '{normalized}' not allowed. Use specific version like '1.85.0' or 'nightly-2024-12-01'");
            return None;
        }

        // Basic validation for nightly format
        if let Some(date_part) = normalized.strip_prefix("nightly-") {
            if date_part.len() != 10 || date_part.matches('-').count() != 2 {
                eprintln!(
                    "Error: Invalid nightly format. Expected 'nightly-YYYY-MM-DD', got '{normalized}'"
                );
                return None;
            }
        }

        Some(normalized)
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

        // if self.docker {
        //     let target_subdir = "docker";
        //     cmd.push("--target-dir".to_string());
        //     cmd.push(format!("target/{}/{}", HELPER_TARGET_SUBDIR, target_subdir));
        // }

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

    /// Generate RUST FLAGS for WASM compilation
    pub fn rust_flags(&self) -> String {
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

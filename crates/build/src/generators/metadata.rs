//! Metadata for deterministic build reproduction

use crate::{command, BuildArgs};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, process::Command};

/// Build metadata for deterministic reproduction
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildMetadata {
    /// Contract info
    pub contract: ContractInfo,

    /// Environment that MUST match for reproduction
    pub environment: Environment,

    /// Build configuration that affects output
    pub build_config: BuildConfig,

    /// Artifacts for verification
    pub artifacts: Artifacts,

    /// Metadata format version
    pub metadata_version: String,
    pub build_timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Environment {
    /// Exact rustc version with commit hash
    pub rustc_version: String,
    /// SDK version (same as fluent-build version)
    pub fluentbase_sdk_version: String,
    /// Build environment
    pub build_platform: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Features enabled
    pub features: Vec<String>,
    /// No default features flag
    pub no_default_features: bool,
    /// Locked dependencies
    pub locked: bool,
    /// Size optimization applied
    pub wasm_opt: bool,
    /// Stack size in bytes
    pub stack_size: u32,
    /// Custom rustflags
    pub rustflags: Vec<String>,
    /// Docker build
    pub docker: bool,
    /// Docker image tag (if docker build)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_tag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Artifacts {
    /// Cargo.lock hash for dependency verification
    pub lockfile_hash: String,

    /// WASM artifact
    pub wasm_hash: String,
    pub wasm_size: u64,

    /// rWASM artifact (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rwasm_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rwasm_size: Option<u64>,

    /// ABI (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abi: Option<Vec<Value>>,
}

pub fn generate(
    contract_dir: &Path,
    args: &BuildArgs,
    wasm_data: &[u8],
    rwasm_data: Option<&[u8]>,
    abi: Option<&crate::generators::solidity::Abi>,
) -> Result<BuildMetadata> {
    // Load package metadata
    let cargo_metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
        .context("Failed to load cargo metadata")?;

    let package = cargo_metadata
        .root_package()
        .ok_or_else(|| anyhow::anyhow!("No root package found"))?;

    // Get lockfile hash - this is critical for reproducibility
    let lockfile_path = contract_dir.join("Cargo.lock");
    let lockfile_hash = if lockfile_path.exists() {
        let lock_content = std::fs::read(&lockfile_path).context("Failed to read Cargo.lock")?;
        calculate_hash(&lock_content)
    } else {
        return Err(anyhow::anyhow!(
            "Cargo.lock not found. Run 'cargo generate-lockfile' first for reproducible builds"
        ));
    };

    // Get SDK version from dependencies
    let fluentbase_sdk_version = cargo_metadata
        .packages
        .iter()
        .find(|p| p.name == "fluentbase-sdk")
        .map(|p| p.version.to_string())
        .ok_or_else(|| anyhow::anyhow!("fluentbase-sdk not found in dependencies"))?;

    // Get actual build platform
    let build_platform = if args.docker {
        get_docker_platform(&args.tag)?
    } else {
        format!("native:{}-{}", std::env::consts::OS, std::env::consts::ARCH)
    };

    // Get actual rustc version from build environment
    let rustc_version = if args.docker {
        get_docker_rustc_version(&args.tag)?
    } else {
        get_rustc_version_detailed()?
    };

    // Calculate artifact hashes
    let (rwasm_hash, rwasm_size) = if let Some(data) = rwasm_data {
        (Some(calculate_hash(data)), Some(data.len() as u64))
    } else {
        (None, None)
    };

    Ok(BuildMetadata {
        contract: ContractInfo {
            name: package.name.clone(),
            version: package.version.to_string(),
        },
        environment: Environment {
            rustc_version,
            fluentbase_sdk_version,
            build_platform,
        },
        build_config: BuildConfig {
            features: args.features.clone(),
            no_default_features: args.no_default_features,
            locked: args.locked,
            wasm_opt: args.wasm_opt,
            stack_size: args.stack_size,
            rustflags: args.rustflags.clone(),
            docker: args.docker,
            docker_tag: if args.docker {
                Some(args.tag.clone())
            } else {
                None
            },
        },
        artifacts: Artifacts {
            lockfile_hash,
            wasm_hash: calculate_hash(wasm_data),
            wasm_size: wasm_data.len() as u64,
            rwasm_hash,
            rwasm_size,
            abi: abi.cloned(),
        },
        metadata_version: "1.0.0".to_string(),
        build_timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

fn get_docker_platform(tag: &str) -> Result<String> {
    let image = command::get_docker_image(tag);

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            &image,
            "sh",
            "-c",
            "echo $(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)",
        ])
        .output()
        .context("Failed to get platform info from Docker")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get Docker platform info");
    }

    let platform = String::from_utf8(output.stdout)?.trim().to_string();

    Ok(format!("docker:{}", platform))
}

fn get_docker_rustc_version(tag: &str) -> Result<String> {
    let image = command::get_docker_image(tag);

    let output = Command::new("docker")
        .args(["run", "--rm", &image, "rustc", "--version"])
        .output()
        .context("Failed to get Rust version from Docker")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get Docker Rust version");
    }

    let version = String::from_utf8(output.stdout)?.trim().to_string();

    // Try to get commit hash
    let verbose_output = Command::new("docker")
        .args(["run", "--rm", &image, "rustc", "--version", "--verbose"])
        .output();

    if let Ok(output) = verbose_output {
        if let Ok(verbose) = String::from_utf8(output.stdout) {
            for line in verbose.lines() {
                if line.starts_with("commit-hash: ") {
                    if let Some(hash) = line.strip_prefix("commit-hash: ") {
                        let short_hash = hash.chars().take(7).collect::<String>();
                        return Ok(format!("{} ({})", version, short_hash));
                    }
                }
            }
        }
    }

    Ok(version)
}

fn get_rustc_version_detailed() -> Result<String> {
    let output = Command::new("rustc")
        .args(&["--version", "--verbose"])
        .output()
        .context("Failed to run rustc")?;

    let output_str = String::from_utf8(output.stdout).context("Invalid UTF-8 in rustc output")?;

    // Extract version line and commit hash
    let mut version = String::new();
    let mut commit = String::new();

    for line in output_str.lines() {
        if version.is_empty() && line.starts_with("rustc") {
            version = line.trim().to_string();
        } else if line.starts_with("commit-hash: ") {
            if let Some(hash) = line.strip_prefix("commit-hash: ") {
                // Take first 7 characters of commit hash
                commit = hash.chars().take(7).collect();
            }
        }
    }

    if !commit.is_empty() {
        Ok(format!("{} ({})", version, commit))
    } else {
        Ok(version)
    }
}

fn calculate_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_hash() {
        let data = b"test data";
        let hash = calculate_hash(data);
        assert_eq!(hash.len(), 64); // SHA256 produces 64 hex characters
    }

    #[test]
    fn test_calculate_hash_deterministic() {
        let data = b"test data";
        let hash1 = calculate_hash(data);
        let hash2 = calculate_hash(data);
        assert_eq!(hash1, hash2); // Same data should produce same hash
    }
}

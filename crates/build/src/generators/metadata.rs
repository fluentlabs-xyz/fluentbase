//! Metadata for deterministic build reproduction

use crate::BuildArgs;
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
    /// Build host platform
    pub host: String,
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

    // Get exact rustc version with commit hash
    let rustc_version = get_rustc_version_detailed()?;

    // Get SDK version from dependencies (this is also fluent-build version)
    let fluentbase_sdk_version = cargo_metadata
        .packages
        .iter()
        .find(|p| p.name == "fluentbase-sdk")
        .map(|p| p.version.to_string())
        .ok_or_else(|| anyhow::anyhow!("fluentbase-sdk not found in dependencies"))?;

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
            host: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        },
        build_config: BuildConfig {
            features: args.features.clone(),
            no_default_features: args.no_default_features,
            locked: args.locked,
            wasm_opt: args.wasm_opt,
            stack_size: args.stack_size,
            rustflags: args.rustflags.clone(),
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

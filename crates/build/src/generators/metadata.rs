//! Metadata for deterministic build reproduction

use crate::{docker, utils::parse_rustc_version};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

// Metadata format version
pub const METADATA_VERSION: &str = "v1.0.0";

// We always build in release mode
pub const PROFILE: &str = "release";

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
    /// Source code information for reproducibility
    pub source: SourceInfo,
    /// Metadata format version
    pub metadata_version: String,
    /// ISO 8601 timestamp
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
    /// Rust toolchain channel from rust-toolchain.toml
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_toolchain: Option<String>,
    /// SDK version info
    pub fluentbase_sdk: SdkInfo,
    /// Build platform (e.g., "docker:linux-x86_64" or "native:macos-aarch64")
    pub build_platform: String,
    /// Host platform (always included for transparency)
    pub host_platform: HostPlatform,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SdkInfo {
    pub version: String,

    /// Git commit hash (full or short)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,

    /// Git tag if specified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_tag: Option<String>,

    /// Git branch if specified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HostPlatform {
    /// OS and architecture
    pub platform: String,
    /// Detailed OS version if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Features enabled
    pub features: Vec<String>,
    /// All available features in the package
    pub available_features: Vec<String>,
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
    /// Docker image info if docker build
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<DockerImageInfo>,
    /// Build profile used
    pub profile: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DockerImageInfo {
    /// Full image name used for build
    pub image_used: String,
    /// Base tag requested
    pub base_tag: String,
    /// Docker image ID for exact reproduction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SourceInfo {
    /// Git repository URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_repository: Option<String>,
    /// Git commit hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    /// Git branch name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Whether working directory was clean
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_dirty: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Artifacts {
    /// Cargo.lock hash for dependency verification
    pub lockfile_hash: String,
    /// WASM artifact
    pub wasm: ArtifactInfo,
    /// rWASM artifact if generated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rwasm: Option<ArtifactInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactInfo {
    /// SHA256 hash
    pub hash: String,
    /// Size in bytes
    pub size: u64,
}

/// Generate build metadata
pub fn generate(
    contract_dir: &Path,
    args: &crate::BuildArgs,
    wasm_data: &[u8],
    rwasm_data: Option<&[u8]>,
    docker_image_used: Option<&str>,
    rust_toolchain: Option<&str>,
) -> Result<BuildMetadata> {
    // Load package metadata
    let cargo_metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(contract_dir.join("Cargo.toml"))
        .exec()
        .context("Failed to load cargo metadata")?;

    let package = cargo_metadata
        .root_package()
        .ok_or_else(|| anyhow::anyhow!("No root package found"))?;

    // Contract info
    let contract = ContractInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
    };

    // Get lockfile hash for reproducibility
    let lockfile_hash = get_lockfile_hash(contract_dir)?;

    // Get SDK info from dependencies
    let fluentbase_sdk = get_sdk_info(&cargo_metadata)?;

    // Get environment info
    let (rustc_version, build_platform) = if let Some(image) = docker_image_used {
        (
            docker::get_image_rustc_version(image)?,
            format!("docker:{}", docker::get_image_platform(image)?),
        )
    } else {
        (
            get_rustc_version_detailed()?,
            format!("native:{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        )
    };

    // Always get host platform for transparency
    let host_platform = HostPlatform {
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        os_version: get_os_version(),
    };

    // Docker image info if applicable
    let docker_image = docker_image_used.map(|image| DockerImageInfo {
        image_used: image.to_string(),
        base_tag: args.docker_tag.clone(),
        image_id: docker::get_image_id(image).ok(),
    });

    // Build config
    let build_config = BuildConfig {
        features: args.features.clone(),
        available_features: package.features.keys().cloned().collect(),
        no_default_features: args.no_default_features,
        locked: args.locked,
        wasm_opt: args.wasm_opt,
        stack_size: args.stack_size,
        rustflags: args.rustflags.clone(),
        docker: args.docker,
        docker_image,
        profile: PROFILE.to_string(),
    };

    // Source info from git (best effort)
    let source = get_source_info(contract_dir)?;

    // Artifacts info
    let artifacts = Artifacts {
        lockfile_hash,
        wasm: ArtifactInfo {
            hash: calculate_hash(wasm_data),
            size: wasm_data.len() as u64,
        },
        rwasm: rwasm_data.map(|data| ArtifactInfo {
            hash: calculate_hash(data),
            size: data.len() as u64,
        }),
    };

    Ok(BuildMetadata {
        contract,
        environment: Environment {
            rustc_version,
            rust_toolchain: rust_toolchain.map(|s| s.to_string()),
            fluentbase_sdk,
            build_platform,
            host_platform,
        },
        build_config,
        source,
        artifacts,
        metadata_version: METADATA_VERSION.to_string(),
        build_timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// Get SHA256 hash of Cargo.lock for reproducibility
fn get_lockfile_hash(contract_dir: &Path) -> Result<String> {
    let lockfile_path = contract_dir.join("Cargo.lock");
    if !lockfile_path.exists() {
        anyhow::bail!(
            "Cargo.lock not found. Run 'cargo generate-lockfile' first for reproducible builds"
        );
    }

    let lock_content = std::fs::read(&lockfile_path).context("Failed to read Cargo.lock")?;

    Ok(calculate_hash(&lock_content))
}

/// Extract SDK info from cargo metadata
fn get_sdk_info(metadata: &cargo_metadata::Metadata) -> Result<SdkInfo> {
    let sdk_package = metadata
        .packages
        .iter()
        .find(|p| p.name == "fluentbase-sdk")
        .ok_or_else(|| anyhow::anyhow!("fluentbase-sdk not found in dependencies"))?;

    let mut sdk_info = SdkInfo {
        version: sdk_package.version.to_string(),
        git_commit: None,
        git_tag: None,
        git_branch: None,
    };

    // Parse git info from source if available
    if let Some(source) = &sdk_package.source {
        if source.repr.starts_with("git+") {
            parse_git_source(&source.repr, &mut sdk_info);
        }
    }

    Ok(sdk_info)
}

/// Parse git source string to extract commit, tag, or branch
fn parse_git_source(source: &str, info: &mut SdkInfo) {
    // Extract full commit hash after '#'
    if let Some(hash_pos) = source.rfind('#') {
        info.git_commit = Some(source[hash_pos + 1..].to_string());
    }

    // Extract tag if present
    if let Some(tag_start) = source.find("?tag=") {
        let tag_end = source[tag_start + 5..]
            .find(['#', '&'])
            .map(|i| tag_start + 5 + i)
            .unwrap_or(source.len());
        info.git_tag = Some(source[tag_start + 5..tag_end].to_string());
    }
    // Extract branch if present
    else if let Some(branch_start) = source.find("?branch=") {
        let branch_end = source[branch_start + 8..]
            .find(['#', '&'])
            .map(|i| branch_start + 8 + i)
            .unwrap_or(source.len());
        info.git_branch = Some(source[branch_start + 8..branch_end].to_string());
    }
    // Extract rev if present (short commit)
    else if let Some(rev_start) = source.find("?rev=") {
        let rev_end = source[rev_start + 5..]
            .find(['#', '&'])
            .map(|i| rev_start + 5 + i)
            .unwrap_or(source.len());
        // If we don't have a full commit hash yet, use the rev as short commit
        if info.git_commit.is_none() {
            info.git_commit = Some(source[rev_start + 5..rev_end].to_string());
        }
    }
}

fn get_source_info(contract_dir: &Path) -> Result<SourceInfo> {
    let mut source = SourceInfo {
        git_repository: None,
        git_commit: None,
        git_branch: None,
        git_dirty: None,
    };

    // Try to get git info (but don't fail if git is not available)
    if contract_dir.join(".git").exists() {
        // Get repository URL
        match Command::new("git")
            .args(["config", "--get", "remote.origin.url"])
            .current_dir(contract_dir)
            .output()
        {
            Ok(output) if output.status.success() => {
                let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !url.is_empty() {
                    source.git_repository = Some(url);
                }
            }
            _ => {}
        }

        // Get current branch
        match Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(contract_dir)
            .output()
        {
            Ok(output) if output.status.success() => {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !branch.is_empty() && branch != "HEAD" {
                    source.git_branch = Some(branch);
                }
            }
            _ => {}
        }

        // Get commit hash
        match Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(contract_dir)
            .output()
        {
            Ok(output) if output.status.success() => {
                source.git_commit =
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
            _ => {}
        }

        // Check if working directory is clean and get dirty files
        match Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(contract_dir)
            .output()
        {
            Ok(output) if output.status.success() => {
                let status_output = String::from_utf8_lossy(&output.stdout);
                let is_dirty = !status_output.is_empty();
                source.git_dirty = Some(is_dirty);
            }
            _ => {}
        }
    }

    Ok(source)
}

/// Get detailed rustc version with commit hash
fn get_rustc_version_detailed() -> Result<String> {
    let output = Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()?;

    Ok(parse_rustc_version(String::from_utf8_lossy(&output.stdout)))
}

/// Get OS version information
fn get_os_version() -> Option<String> {
    Some(format!(
        "{} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

/// Calculate SHA256 hash
fn calculate_hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

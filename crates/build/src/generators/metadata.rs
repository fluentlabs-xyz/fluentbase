//! Metadata for deterministic build reproduction

use crate::{command, utils, BuildArgs};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

    /// SDK/Build version with git info
    pub fluentbase_sdk: FluentbaseInfo,

    /// Build environment
    pub build_platform: String,

    /// Host information for native builds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_info: Option<HostInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FluentbaseInfo {
    /// Package version from Cargo.toml
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
pub struct HostInfo {
    /// OS version/release
    pub os_version: String,
    /// CPU architecture details
    pub cpu_info: String,
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
    /// Docker image info (if docker build)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<DockerImageInfo>,
    /// Build profile used
    pub profile: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DockerImageInfo {
    /// Base image tag requested
    pub base_tag: String,
    /// Full image name actually used
    pub image_used: String,
    /// Docker image ID for exact reproduction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
    /// Whether cache image was created
    pub cache_created: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Git repository URL (origin)
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

    /// List of uncommitted files (if dirty)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_dirty_files: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Artifacts {
    /// Cargo.lock hash for dependency verification
    pub lockfile_hash: String,

    /// WASM artifact
    pub wasm: ArtifactInfo,

    /// rWASM artifact (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rwasm: Option<ArtifactInfo>,

    /// ABI (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abi: Option<Vec<Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactInfo {
    /// SHA256 hash
    pub hash: String,
    /// Size in bytes
    pub size: u64,
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

    // Get SDK version and git info from dependencies
    let fluentbase_info = extract_fluentbase_info(&cargo_metadata)
        .ok_or_else(|| anyhow::anyhow!("fluentbase-sdk not found in dependencies"))?;

    // Get Rust toolchain info
    let rust_toolchain = utils::find_rust_toolchain_version(contract_dir)?;

    // Get actual build environment info
    let (build_platform, rustc_version, docker_image, host_info) = if args.docker {
        let actual_image = get_actual_docker_image(args, contract_dir)?;
        let platform = get_docker_platform(&actual_image)?;
        let rustc = get_docker_rustc_version(&actual_image)?;
        let image_id = get_docker_image_id(&actual_image).ok();

        let docker_info = DockerImageInfo {
            base_tag: args.tag.clone(),
            image_used: actual_image.clone(),
            image_id,
            cache_created: actual_image.contains(command::CACHE_IMAGE_PREFIX),
        };

        (platform, rustc, Some(docker_info), None)
    } else {
        let platform = format!("native:{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let rustc = get_rustc_version_detailed()?;
        let host = get_host_info();
        (platform, rustc, None, host)
    };

    // Get source info
    let source = get_source_info(contract_dir)?;

    // Get available features
    let available_features: Vec<String> = package.features.keys().cloned().collect();

    // Calculate artifact info
    let wasm_info = ArtifactInfo {
        hash: calculate_hash(wasm_data),
        size: wasm_data.len() as u64,
    };

    let rwasm_info = rwasm_data.map(|data| ArtifactInfo {
        hash: calculate_hash(data),
        size: data.len() as u64,
    });

    // Extract contract info
    let contract = ContractInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
    };

    Ok(BuildMetadata {
        contract,
        environment: Environment {
            rustc_version,
            rust_toolchain,
            fluentbase_sdk: fluentbase_info,
            build_platform,
            host_info,
        },
        build_config: BuildConfig {
            features: args.features.clone(),
            available_features,
            no_default_features: args.no_default_features,
            locked: args.locked,
            wasm_opt: args.wasm_opt,
            stack_size: args.stack_size,
            rustflags: args.rustflags.clone(),
            docker: args.docker,
            docker_image,
            profile: PROFILE.to_string(),
        },
        source,
        artifacts: Artifacts {
            lockfile_hash,
            wasm: wasm_info,
            rwasm: rwasm_info,
            abi: abi.cloned(),
        },
        metadata_version: METADATA_VERSION.to_string(),
        build_timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

fn extract_fluentbase_info(metadata: &cargo_metadata::Metadata) -> Option<FluentbaseInfo> {
    metadata
        .packages
        .iter()
        .find(|p| p.name == "fluentbase-sdk")
        .map(|package| {
            let mut info = FluentbaseInfo {
                version: package.version.to_string(),
                git_commit: None,
                git_tag: None,
                git_branch: None,
            };

            if let Some(source) = &package.source {
                parse_git_source(&source.repr, &mut info);
            }

            info
        })
}

fn parse_git_source(source_str: &str, info: &mut FluentbaseInfo) {
    // Only process git sources
    if !source_str.starts_with("git+") {
        return;
    }

    // Extract full commit hash after '#'
    if let Some(hash_pos) = source_str.rfind('#') {
        info.git_commit = Some(source_str[hash_pos + 1..].to_string());
    }

    // Extract tag if present
    if let Some(tag_start) = source_str.find("?tag=") {
        let tag_end = source_str[tag_start + 5..]
            .find(['#', '&'])
            .map(|i| tag_start + 5 + i)
            .unwrap_or(source_str.len());
        info.git_tag = Some(source_str[tag_start + 5..tag_end].to_string());
    }
    // Extract branch if present
    else if let Some(branch_start) = source_str.find("?branch=") {
        let branch_end = source_str[branch_start + 8..]
            .find(['#', '&'])
            .map(|i| branch_start + 8 + i)
            .unwrap_or(source_str.len());
        info.git_branch = Some(source_str[branch_start + 8..branch_end].to_string());
    }
    // Extract rev if present (short commit)
    else if let Some(rev_start) = source_str.find("?rev=") {
        let rev_end = source_str[rev_start + 5..]
            .find(['#', '&'])
            .map(|i| rev_start + 5 + i)
            .unwrap_or(source_str.len());
        // If we don't have a full commit hash yet, use the rev as short commit
        if info.git_commit.is_none() {
            info.git_commit = Some(source_str[rev_start + 5..rev_end].to_string());
        }
    }
}

/// Determine the actual Docker image that will be used
fn get_actual_docker_image(args: &BuildArgs, contract_dir: &Path) -> Result<String> {
    let base_image = command::get_docker_image(&args.tag);

    // Check if we need a custom Rust version
    let rust_version = args.get_rust_version(contract_dir);

    if let Some(rust_version) = rust_version {
        // This will match the logic in command.rs
        let sanitized_tag = args.tag.replace(['/', ':'], "-");
        let cache_image = format!(
            "{}-{}-rust-{}",
            command::CACHE_IMAGE_PREFIX,
            sanitized_tag,
            rust_version
        );

        // Check if cache image exists or will be created
        if command::image_exists(&cache_image)? {
            return Ok(cache_image);
        }

        // Check if base image has the right version
        if let Ok(base_rust_version) = get_docker_rustc_version(&base_image) {
            // Extract just the version number for comparison
            let base_version = base_rust_version.split_whitespace().nth(1).unwrap_or("");
            if base_version == rust_version {
                return Ok(base_image);
            }
        }

        // Cache image will be created
        Ok(cache_image)
    } else {
        Ok(base_image)
    }
}

fn get_source_info(contract_dir: &Path) -> Result<SourceInfo> {
    let mut source = SourceInfo {
        git_repository: None,
        git_commit: None,
        git_branch: None,
        git_dirty: None,
        git_dirty_files: None,
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

                if is_dirty {
                    let dirty_files: Vec<String> = status_output
                        .lines()
                        .map(|line| {
                            // Remove status prefix (e.g., "M ", "?? ")
                            line.split_at(3).1.trim().to_string()
                        })
                        .collect();
                    source.git_dirty_files = Some(dirty_files);
                }
            }
            _ => {}
        }
    }

    Ok(source)
}

fn get_host_info() -> Option<HostInfo> {
    // Try to get OS version
    let os_version = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.starts_with("PRETTY_NAME="))
                    .map(|line| {
                        line.trim_start_matches("PRETTY_NAME=")
                            .trim_matches('"')
                            .to_string()
                    })
            })
    } else if cfg!(target_os = "macos") {
        Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    Some(format!(
                        "macOS {}",
                        String::from_utf8_lossy(&output.stdout).trim()
                    ))
                } else {
                    None
                }
            })
    } else if cfg!(target_os = "windows") {
        Some(format!("Windows {}", std::env::consts::ARCH))
    } else {
        None
    };

    // Get CPU info
    let cpu_info = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.starts_with("model name"))
                    .map(|line| line.split(':').nth(1).unwrap_or("").trim().to_string())
            })
    } else if cfg!(target_os = "macos") {
        Command::new("sysctl")
            .arg("-n")
            .arg("machdep.cpu.brand_string")
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                } else {
                    None
                }
            })
    } else {
        None
    };

    match (os_version, cpu_info) {
        (Some(os), Some(cpu)) => Some(HostInfo {
            os_version: os,
            cpu_info: cpu,
        }),
        (Some(os), None) => Some(HostInfo {
            os_version: os,
            cpu_info: std::env::consts::ARCH.to_string(),
        }),
        _ => None,
    }
}

fn get_docker_platform(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            image,
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

fn get_docker_rustc_version(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version"])
        .output()
        .context("Failed to get Rust version from Docker")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get Docker Rust version");
    }

    let version = String::from_utf8(output.stdout)?.trim().to_string();

    // Try to get commit hash
    let verbose_output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version", "--verbose"])
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

fn get_docker_image_id(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.Id}}", image])
        .output()
        .context("Failed to inspect Docker image")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get Docker image ID");
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn get_rustc_version_detailed() -> Result<String> {
    let output = Command::new("rustc")
        .args(["--version", "--verbose"])
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
    fn test_parse_git_source_with_rev() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        let source = "git+https://github.com/fluentlabs-xyz/fluentbase?rev=c6fe78da#c6fe78dabf1234567890abcdef1234567890abcd";
        parse_git_source(source, &mut info);

        assert_eq!(
            info.git_commit,
            Some("c6fe78dabf1234567890abcdef1234567890abcd".to_string())
        );
        assert_eq!(info.git_tag, None);
        assert_eq!(info.git_branch, None);
    }

    #[test]
    fn test_parse_git_source_with_tag() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        let source = "git+https://github.com/fluentlabs-xyz/fluentbase?tag=v0.2.1-dev#bdadc577abcdef1234567890abcdef1234567890";
        parse_git_source(source, &mut info);

        assert_eq!(
            info.git_commit,
            Some("bdadc577abcdef1234567890abcdef1234567890".to_string())
        );
        assert_eq!(info.git_tag, Some("v0.2.1-dev".to_string()));
        assert_eq!(info.git_branch, None);
    }

    #[test]
    fn test_parse_git_source_with_branch() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        let source = "git+https://github.com/fluentlabs-xyz/fluentbase?branch=main#abc123def4567890abcdef1234567890abcdef12";
        parse_git_source(source, &mut info);

        assert_eq!(
            info.git_commit,
            Some("abc123def4567890abcdef1234567890abcdef12".to_string())
        );
        assert_eq!(info.git_tag, None);
        assert_eq!(info.git_branch, Some("main".to_string()));
    }

    #[test]
    fn test_parse_git_source_non_git() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        let source = "registry+https://github.com/rust-lang/crates.io-index";
        parse_git_source(source, &mut info);

        // Should not modify anything for non-git sources
        assert_eq!(info.git_commit, None);
        assert_eq!(info.git_tag, None);
        assert_eq!(info.git_branch, None);
    }

    #[test]
    fn test_parse_git_source_without_commit() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        // Edge case: git source without commit hash
        let source = "git+https://github.com/fluentlabs-xyz/fluentbase?branch=main";
        parse_git_source(source, &mut info);

        assert_eq!(info.git_commit, None);
        assert_eq!(info.git_tag, None);
        assert_eq!(info.git_branch, Some("main".to_string()));
    }

    #[test]
    fn test_parse_git_source_with_additional_params() {
        let mut info = FluentbaseInfo {
            version: "0.1.0".to_string(),
            git_commit: None,
            git_tag: None,
            git_branch: None,
        };

        // Source with additional parameters
        let source = "git+https://github.com/fluentlabs-xyz/fluentbase?tag=v1.0.0&feature=foo#deadbeef1234567890abcdef1234567890abcdef";
        parse_git_source(source, &mut info);

        assert_eq!(
            info.git_commit,
            Some("deadbeef1234567890abcdef1234567890abcdef".to_string())
        );
        assert_eq!(info.git_tag, Some("v1.0.0".to_string()));
        assert_eq!(info.git_branch, None);
    }
}

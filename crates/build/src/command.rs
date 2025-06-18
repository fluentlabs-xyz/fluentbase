use crate::utils::versions_match;
use anyhow::{bail, Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DOCKER_IMAGE_REGISTRY: &str = "ghcr.io/fluentlabs-xyz/fluentbase-build";
const DOCKER_IMAGE_ENV_VAR: &str = "FLUENT_DOCKER_IMAGE";
const DOCKER_PLATFORM: &str = "linux/amd64";
pub(crate) const CACHE_IMAGE_PREFIX: &str = "fluentbase-build";

/// Run command in Docker or locally
pub fn run(args: &[String], work_dir: &Path, docker_config: Option<DockerConfig>) -> Result<()> {
    let Some(config) = docker_config else {
        // Run locally
        return run_local(args, work_dir);
    };

    // Run in Docker
    check_docker()?;
    let image = ensure_image(&config.sdk_tag, config.rust_version.as_deref(), work_dir)?;
    run_docker(args, work_dir, &image, &config.env_vars, &config.mount_dir)
}

#[derive(Debug)]
pub struct DockerConfig {
    pub sdk_tag: String,
    pub rust_version: Option<String>,
    pub env_vars: Vec<(String, String)>,
    pub mount_dir: PathBuf,
}

// ============================================================
// Implementation
// ============================================================

fn run_local(args: &[String], work_dir: &Path) -> Result<()> {
    let (cmd, args) = args
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty command"))?;

    let status = Command::new(cmd)
        .args(args)
        .current_dir(work_dir)
        .status()
        .with_context(|| format!("Failed to execute: {}", cmd))?;

    if !status.success() {
        bail!("Command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

fn run_docker(
    args: &[String],
    work_dir: &Path,
    image: &str,
    env_vars: &[(String, String)],
    mount_dir: &Path,
) -> Result<()> {
    let mount_dir = mount_dir
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize mount dir: {}", mount_dir.display()))?;

    let work_dir = work_dir
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize work dir: {}", work_dir.display()))?;

    let relative_dir = work_dir.strip_prefix(&mount_dir).with_context(|| {
        format!(
            "Work dir {} is not within mount dir {}",
            work_dir.display(),
            mount_dir.display()
        )
    })?;

    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "--platform",
        DOCKER_PLATFORM,
        "-v",
        &format!("{}:/workspace", mount_dir.display()),
        "-v",
        "cargo-registry:/usr/local/cargo/registry",
        "-v",
        "cargo-git:/usr/local/cargo/git",
        "-w",
        &format!("/workspace/{}", relative_dir.display()),
    ]);

    // Add environment variables
    for (key, value) in env_vars {
        cmd.args(["-e", &format!("{}={}", key, value)]);
    }

    // Add image and command
    cmd.arg(image);
    cmd.args(args);

    let status = cmd.status().context("Failed to execute Docker command")?;

    if !status.success() {
        bail!("Docker command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Normalize Rust version for rustup (e.g., "1.87" -> "1.87.0")
fn normalize_rust_version_for_rustup(version: &str) -> String {
    let version = version.trim_start_matches("rustc ").trim();
    let parts: Vec<&str> = version.split('.').collect();

    match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => version.to_string(),
    }
}

/// Normalize version for cache image naming (e.g., "1.87.0" -> "1.87")
fn normalize_version_for_cache(version: &str) -> String {
    let version = version.trim_start_matches("rustc ").trim();
    let parts: Vec<&str> = version.split('.').collect();

    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version.to_string()
    }
}

fn ensure_image(sdk_tag: &str, rust_version: Option<&str>, work_dir: &Path) -> Result<String> {
    let base_image = get_docker_image(sdk_tag);

    // First, ensure base image exists
    ensure_base_image_exists(&base_image)?;

    // Get Rust version if not specified
    let rust_version = match rust_version {
        Some(v) => {
            // Validate the version format
            crate::utils::validate_rust_version(v)?;
            Some(v.to_string())
        }
        None => {
            // Try to detect from rust-toolchain.toml
            crate::utils::find_rust_toolchain_version(work_dir)?
        }
    };

    // No specific Rust version? Use base image
    let Some(rust_version) = rust_version else {
        println!("Using base Docker image: {}", base_image);
        return Ok(base_image);
    };

    // Check if base image already has compatible Rust version
    if let Ok(base_rust_version) = get_rust_version_from_image(&base_image) {
        if versions_match(&base_rust_version, &rust_version) {
            println!(
                "Using Docker image: {} (Rust {} ✓)",
                base_image, base_rust_version
            );
            return Ok(base_image);
        }
        println!(
            "Base image has Rust {}, but project needs Rust {}",
            base_rust_version, rust_version
        );
    }

    // Check if we already have a cached image with compatible version
    // Use normalized version for cache naming to avoid duplicates
    let cache_version = normalize_version_for_cache(&rust_version);
    let cache_image = format_cache_image_name(sdk_tag, &cache_version);

    // Check if cached image exists and has compatible version
    if image_exists(&cache_image)? {
        if let Ok(cached_rust_version) = get_rust_version_from_image(&cache_image) {
            if versions_match(&cached_rust_version, &rust_version) {
                println!(
                    "Using cached image: {} (Rust {} ✓)",
                    cache_image, cached_rust_version
                );
                return Ok(cache_image);
            }
        }
    }

    // Build new cached image with specific Rust version
    println!(
        "Building cached image with Rust {} toolchain...",
        rust_version
    );

    // Use full version for rustup
    let rustup_version = normalize_rust_version_for_rustup(&rust_version);
    build_with_rust(&base_image, &cache_image, &rustup_version, &rust_version)?;

    Ok(cache_image)
}

fn ensure_base_image_exists(image: &str) -> Result<()> {
    // First check locally
    if image_exists(image)? {
        return Ok(());
    }

    // If it's a local image, don't try to pull
    if is_local_image(image) {
        bail!(
            "Local Docker image '{}' not found.\n\
             \n\
             To fix this, either:\n\
             1. Build the image locally\n\
             2. Use a registry image with --tag\n\
             3. Set FLUENT_DOCKER_IMAGE to a valid image",
            image
        );
    }

    // Try to pull from registry
    println!("Pulling image: {} (this may take a few minutes)...", image);
    let status = Command::new("docker")
        .args(["pull", "--platform", DOCKER_PLATFORM, image])
        .status()
        .context("Failed to pull image")?;

    if !status.success() {
        bail!(
            "Failed to pull image: {}\n\
             \n\
             This might be due to:\n\
             1. Network connectivity issues\n\
             2. Image not found in registry\n\
             3. Authentication required\n\
             \n\
             Try running: docker pull {}",
            image,
            image
        );
    }

    Ok(())
}

fn build_with_rust(
    base: &str,
    target: &str,
    rustup_version: &str,
    label_version: &str,
) -> Result<()> {
    println!(
        "Building Docker image with Rust {} toolchain (one-time setup)...",
        label_version
    );
    println!("This may take a few minutes on first run.");

    let dockerfile = format!(
        r#"FROM {}
RUN rustup toolchain install {}-x86_64-unknown-linux-gnu
RUN rustup default {}-x86_64-unknown-linux-gnu
RUN rustup target add wasm32-unknown-unknown
RUN rustup component add rust-src --toolchain {}-x86_64-unknown-linux-gnu
LABEL rust.version="{}"
LABEL fluentbase.build.cache="true"
"#,
        base, rustup_version, rustup_version, rustup_version, label_version
    );

    let mut child = Command::new("docker")
        .args([
            "build",
            "--platform",
            DOCKER_PLATFORM,
            "-t",
            target,
            "-f-",
            ".",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to start Docker build")?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(dockerfile.as_bytes())?;

    let status = child.wait()?;
    if !status.success() {
        bail!("Failed to build Docker image with Rust {}", label_version);
    }

    println!("Successfully built cached image: {}", target);
    Ok(())
}

// ============================================================
// Utilities
// ============================================================

fn check_docker() -> Result<()> {
    let output = Command::new("docker").args(["version"]).output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => bail!("Docker command failed. Is Docker daemon running?"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Docker not found in PATH.\n\
                 \n\
                 Fluentbase builds run in Docker by default to ensure reproducible builds.\n\
                 Please install Docker from https://docker.com or use --no-docker for local builds.\n\
                 \n\
                 Note: Local builds may not be reproducible across different environments."
            )
        }
        Err(e) => Err(e).context("Failed to check Docker installation"),
    }
}

pub(crate) fn image_exists(image: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["images", "-q", image])
        .output()
        .context("Failed to check Docker images")?;

    Ok(!output.stdout.is_empty())
}

fn get_rust_version_from_image(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version"])
        .output()
        .context("Failed to get Rust version from image")?;

    if !output.status.success() {
        bail!("Failed to get Rust version from image: {}", image);
    }

    let version_output = String::from_utf8_lossy(&output.stdout);
    let version = version_output
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse Rust version"))?
        .to_string();

    Ok(version)
}

pub(crate) fn get_docker_image(tag: &str) -> String {
    std::env::var(DOCKER_IMAGE_ENV_VAR)
        .unwrap_or_else(|_| format!("{}:{}", DOCKER_IMAGE_REGISTRY, tag))
}

fn is_local_image(image: &str) -> bool {
    // Local images don't have registry prefix or have specific prefixes
    !image.contains("ghcr.io/")
        && !image.contains("docker.io/")
        && (!image.contains('/') || image.starts_with("local/"))
}

fn format_cache_image_name(sdk_tag: &str, rust_version: &str) -> String {
    // Sanitize the tag to remove special characters that might cause issues
    let sanitized_tag = sdk_tag.replace(['/', ':'], "-");
    format!(
        "{}-{}-rust-{}",
        CACHE_IMAGE_PREFIX, sanitized_tag, rust_version
    )
}

// TODO(d1r1): setup cache policy cleanup
#[allow(dead_code)]
/// List all cached Docker images created by fluentbase-build
fn list_cached_images() -> Result<Vec<String>> {
    let output = Command::new("docker")
        .args([
            "images",
            "--format",
            "{{.Repository}}:{{.Tag}}",
            "--filter",
            &format!("reference={}*", CACHE_IMAGE_PREFIX),
        ])
        .output()
        .context("Failed to list Docker images")?;

    if !output.status.success() {
        bail!("Failed to list cached images");
    }

    let images = String::from_utf8(output.stdout)?
        .lines()
        .map(String::from)
        .filter(|s| !s.is_empty())
        .collect();

    Ok(images)
}

#[allow(dead_code)]
/// Remove cached Docker images created by fluentbase-build
fn clean_cached_images() -> Result<()> {
    let images = list_cached_images()?;

    if images.is_empty() {
        println!("No cached images found.");
        return Ok(());
    }

    println!("Found {} cached image(s):", images.len());
    for image in &images {
        println!("  - {}", image);
    }

    // Remove each image
    for image in images {
        println!("Removing {}...", image);
        let status = Command::new("docker")
            .args(["rmi", &image])
            .status()
            .context("Failed to remove image")?;

        if !status.success() {
            eprintln!("Warning: Failed to remove {}", image);
        }
    }

    println!("Cache cleanup complete.");
    Ok(())
}

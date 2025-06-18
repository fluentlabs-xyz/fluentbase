// command.rs - Simple Docker command execution

use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DOCKER_IMAGE_REGISTRY: &str = "ghcr.io/fluentlabs/fluent-build";
const DOCKER_IMAGE_ENV_VAR: &str = "FLUENT_DOCKER_IMAGE";
const DOCKER_PLATFORM: &str = "linux/amd64";
const CACHE_IMAGE_PREFIX: &str = "fluent-build-cache";

/// Run command in Docker or locally
pub fn run(args: &[String], work_dir: &Path, docker_config: Option<DockerConfig>) -> Result<()> {
    let Some(config) = docker_config else {
        // Run locally
        return run_local(args, work_dir);
    };

    // Run in Docker
    check_docker()?;
    let image = ensure_image(&config.sdk_tag, config.rust_version.as_deref())?;
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
        anyhow::bail!("Command failed with exit code: {:?}", status.code());
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
        anyhow::bail!("Docker command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

fn ensure_image(sdk_tag: &str, rust_version: Option<&str>) -> Result<String> {
    let base_image = get_docker_image(sdk_tag);

    // First, ensure base image exists
    ensure_base_image_exists(&base_image)?;

    // No specific Rust version? Use base image
    let Some(rust_version) = rust_version else {
        return Ok(base_image);
    };

    // Check if base image already has this Rust version
    // Only check if it's not a local/custom image
    if !is_local_image(&base_image) {
        if let Ok(version) = get_rust_version(&base_image) {
            if version == rust_version {
                return Ok(base_image);
            }
        }
    } else {
        // For local images, assume they have the correct Rust version
        println!("Using local image: {}", base_image);
        return Ok(base_image);
    }

    // Build cached image with specific Rust version
    let cache_image = format!("{}-{}-rust-{}", CACHE_IMAGE_PREFIX, sdk_tag, rust_version);

    if !image_exists(&cache_image)? {
        build_with_rust(&base_image, &cache_image, rust_version)?;
    }

    Ok(cache_image)
}

fn ensure_base_image_exists(image: &str) -> Result<()> {
    // First check locally
    if image_exists(image)? {
        return Ok(());
    }

    // If it's a local image, don't try to pull
    if is_local_image(image) {
        anyhow::bail!(
            "Local Docker image '{}' not found. Please build it first.",
            image
        );
    }

    // Try to pull from registry
    println!("Pulling image: {}", image);
    let status = Command::new("docker")
        .args(["pull", "--platform", DOCKER_PLATFORM, image])
        .status()
        .context("Failed to pull image")?;

    if !status.success() {
        anyhow::bail!("Failed to pull image: {}", image);
    }

    Ok(())
}

fn build_with_rust(base: &str, target: &str, rust_version: &str) -> Result<()> {
    println!(
        "Building image with Rust {} (one-time setup)...",
        rust_version
    );

    let dockerfile = format!(
        r#"FROM {}
RUN rustup toolchain install {} && \
    rustup default {} && \
    rustup target add wasm32-unknown-unknown
LABEL rust.version={}
"#,
        base, rust_version, rust_version, rust_version
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
        anyhow::bail!("Failed to build Docker image");
    }

    Ok(())
}

// ============================================================
// Utilities
// ============================================================

fn check_docker() -> Result<()> {
    Command::new("docker")
        .args(["version"])
        .output()
        .context("Docker not found. Please install Docker")?;
    Ok(())
}

fn image_exists(image: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["images", "-q", image])
        .output()
        .context("Failed to check Docker images")?;

    Ok(!output.stdout.is_empty())
}

fn get_rust_version(image: &str) -> Result<String> {
    // Don't call pull_if_needed here - image should already exist
    let output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version"])
        .output()
        .context("Failed to get Rust version from image")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get Rust version from image: {}", image);
    }

    let version = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
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

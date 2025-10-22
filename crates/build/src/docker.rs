use crate::utils::parse_rustc_version;
use crate::{DOCKER_PLATFORM, CARGO_CACHE_VOLUME};
use anyhow::{bail, Context, Result};
use std::{path::Path, process::Command};

/// Run command in the Docker container
pub fn run_in_docker(
    image: &str,
    args: &[String],
    mount_dir: &Path,
    work_dir: &Path,
    env_vars: &[(String, String)],
    rust_toolchain: &Option<String>,
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
        &format!("{}:/usr/local/cargo", CARGO_CACHE_VOLUME),
        "-v",
        &format!("{}:/workspace", mount_dir.display()),
        "-w",
        &format!("/workspace/{}", relative_dir.display()),
    ]);

    // Add environment variables
    for (key, value) in env_vars {
        cmd.args(["-e", &format!("{key}={value}")]);
    }

    // Set the rust toolchain ONLY if it's explicitly provided.
    // If it's None, the container's default toolchain will be used.
    if let Some(toolchain) = rust_toolchain {
        cmd.args(["-e", &format!("RUSTUP_TOOLCHAIN={toolchain}")]);
    }

    cmd.arg(image);
    cmd.args(args);

    let status = cmd.status().context("Failed to execute Docker command")?;

    if !status.success() {
        bail!("Docker command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Get Docker image for builds
pub fn ensure_rust_image(image: &str) -> Result<String> {
    check_docker()?;
    verify_host_platform()?;

    // Ensure image exists (pull if needed)
    if !image_exists(image)? {
        println!("Pulling base image: {image} ...");
        let status = Command::new("docker")
            .args(["pull", "--platform", DOCKER_PLATFORM, image])
            .status()?;
        if !status.success() {
            bail!("Failed to get image: {}", image);
        }
    }

    println!("Using image: {image}");
    Ok(image.to_string())
}

/// PUBLIC UTILS
/// Get Rust toolchain version from Docker image
pub fn get_image_rustc_version(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version", "--verbose"])
        .output()
        .context("Failed to get Rust version from Docker image")?;

    if !output.status.success() {
        bail!("Failed to get Rust version from image: {}", image);
    }

    Ok(parse_rustc_version(String::from_utf8_lossy(&output.stdout)))
}

/// Get platform information from Docker image
pub fn get_image_platform(image: &str) -> Result<String> {
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
        .context("Failed to get platform info from Docker image")?;

    if !output.status.success() {
        bail!("Failed to get platform info from image: {}", image);
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Get Docker image ID for exact reproduction
pub fn get_image_id(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.Id}}", image])
        .output()
        .context("Failed to inspect Docker image")?;

    if !output.status.success() {
        bail!("Failed to get image ID for: {}", image);
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

// Helper functions

fn check_docker() -> Result<()> {
    let output = Command::new("docker").args(["version"]).output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => bail!("Docker command failed. Is Docker daemon running?"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Docker not found in PATH.\n\
                \n\
                Please install Docker from https://docker.com or use --no-docker for local builds."
            )
        }
        Err(e) => Err(e).context("Failed to check Docker installation"),
    }
}

fn verify_host_platform() -> Result<()> {
    // Windows requires WSL2 for linux/amd64 platform builds
    #[cfg(target_os = "windows")]
    {
        let in_wsl = std::env::var("WSL_DISTRO_NAME").is_ok()
            || std::path::Path::new("/proc/version").exists();

        if !in_wsl {
            bail!(
                "Docker builds on Windows require WSL2.\n\
                \n\
                Fluentbase builds target linux/amd64 platform for reproducibility.\n\
                Please run this command inside WSL2 or use --no-docker for local builds.\n\
                \n\
                Note: Local builds may not be reproducible across different platforms."
            );
        }
    }

    Ok(())
}

fn image_exists(image: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["images", "-q", image])
        .output()
        .context("Failed to check Docker images")?;

    Ok(!output.stdout.is_empty())
}

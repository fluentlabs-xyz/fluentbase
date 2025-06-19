use crate::utils::parse_rustc_version;
use anyhow::{bail, Context, Result};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const DOCKER_IMAGE_REGISTRY: &str = "ghcr.io/fluentlabs-xyz/fluentbase-build";
const DOCKER_IMAGE_ENV_VAR: &str = "FLUENT_DOCKER_IMAGE";
const DOCKER_PLATFORM: &str = "linux/amd64";

/// Run command in Docker container
pub fn run_in_docker(
    image: &str,
    args: &[String],
    mount_dir: &Path,
    work_dir: &Path,
    env_vars: &[(String, String)],
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

    cmd.arg(image);
    cmd.args(args);

    let status = cmd.status().context("Failed to execute Docker command")?;

    if !status.success() {
        bail!("Docker command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Get or create Docker image with specific Rust toolchain
pub fn ensure_rust_image(base_tag: &str, rust_toolchain: Option<&str>) -> Result<String> {
    check_docker()?;
    verify_host_platform()?;

    let base_image = get_base_image(base_tag);
    // Ensure base image exists (pull if needed)
    if !image_exists(&base_image)? {
        println!("Pulling base image: {} ...", base_image);
        let status = Command::new("docker")
            .args(["pull", "--platform", DOCKER_PLATFORM, &base_image])
            .status()?;
        if !status.success() {
            bail!("Failed to get base image: {}", base_image);
        }
    }

    // No specific toolchain? Use base image
    let Some(requested_toolchain) = rust_toolchain else {
        println!("Using base image: {}", base_image);
        return Ok(base_image);
    };

    let normalized_toolchain = normalize_toolchain_for_rustup(requested_toolchain);

    // Check if base image already has the right toolchain
    if let Ok(base_toolchain) = get_image_toolchain(&base_image) {
        if toolchain_compatible(&base_toolchain, &normalized_toolchain) {
            println!(
                "Using base image: {} (Rust {} ✓)",
                base_image, base_toolchain
            );
            return Ok(base_image);
        }
        println!(
            "Base image has Rust {}, but project needs Rust {}",
            base_toolchain, normalized_toolchain
        );
    }

    // Need different toolchain - use cached image
    let cache_image = format!(
        "fluentbase-cache-{}-rust-{}",
        base_tag.replace(['/', ':'], "-"),
        normalized_toolchain.replace('.', "_")
    );

    if !image_exists(&cache_image)? {
        println!(
            "Building image with Rust {} toolchain (one-time setup)...",
            normalized_toolchain
        );
        create_toolchain_image(&base_image, &cache_image, &normalized_toolchain)?;
    } else {
        println!(
            "Using cached image: {} (Rust {})",
            cache_image, normalized_toolchain
        );
    }

    Ok(cache_image)
}

#[allow(dead_code)]
/// Clean up cached Docker images
pub fn clean_cached_images() -> Result<()> {
    let output = Command::new("docker")
        .args([
            "images",
            "--format",
            "{{.Repository}}:{{.Tag}}",
            "--filter",
            "reference=fluentbase-cache-*",
        ])
        .output()
        .context("Failed to list Docker images")?;

    if !output.status.success() {
        bail!("Failed to list cached images");
    }

    let images: Vec<String> = String::from_utf8(output.stdout)?
        .lines()
        .map(String::from)
        .filter(|s| !s.is_empty())
        .collect();

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

fn get_base_image(tag: &str) -> String {
    std::env::var(DOCKER_IMAGE_ENV_VAR)
        .unwrap_or_else(|_| format!("{}:{}", DOCKER_IMAGE_REGISTRY, tag))
}

fn image_exists(image: &str) -> Result<bool> {
    let output = Command::new("docker")
        .args(["images", "-q", image])
        .output()
        .context("Failed to check Docker images")?;

    Ok(!output.stdout.is_empty())
}

fn get_image_toolchain(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["run", "--rm", image, "rustc", "--version"])
        .output()
        .context("Failed to get Rust toolchain from image")?;

    if !output.status.success() {
        bail!("Failed to get Rust toolchain from image: {}", image);
    }

    // Parse "rustc 1.87.0 (stable ...)" -> "1.87.0"
    let version_output = String::from_utf8_lossy(&output.stdout);
    let version = version_output
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse Rust version"))?
        .to_string();

    Ok(version)
}
fn create_toolchain_image(base: &str, target: &str, toolchain: &str) -> Result<()> {
    // Normalize toolchain for rustup (expects full version like 1.87.0)
    let rustup_toolchain = normalize_toolchain_for_rustup(toolchain);

    let toolchain_with_arch = format!("{}-x86_64-unknown-linux-gnu", rustup_toolchain);
    println!("Toolchain with architecture: {}", toolchain_with_arch);

    let dockerfile = format!(
        r#"ARG BUILD_PLATFORM=linux/amd64
FROM --platform=${{BUILD_PLATFORM}} {}
RUN rustup toolchain install {}
RUN rustup default {}
RUN rustup target add wasm32-unknown-unknown
RUN rustup component add rust-src --toolchain {}
LABEL rust.toolchain="{}"
"#,
        base, toolchain_with_arch, toolchain_with_arch, toolchain_with_arch, toolchain
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
        bail!("Failed to build Docker image with Rust {}", toolchain);
    }

    println!("Successfully built cached image: {}", target);
    Ok(())
}

fn toolchain_compatible(installed: &str, requested: &str) -> bool {
    // For nightly and beta versions - must match exactly
    if installed.starts_with("nightly-") || requested.starts_with("nightly-") {
        return installed == requested;
    }

    if installed.starts_with("beta-") || requested.starts_with("beta-") {
        return installed == requested;
    }

    // For stable versions
    let installed_parts: Vec<&str> = installed.split('.').collect();
    let requested_parts: Vec<&str> = requested.split('.').collect();

    // Must have at least major.minor
    if installed_parts.len() < 2 || requested_parts.len() < 2 {
        return false;
    }

    // Major and minor must match
    if installed_parts[0] != requested_parts[0] || installed_parts[1] != requested_parts[1] {
        return false;
    }

    // If requested has no patch version (e.g., "1.77"), any patch version is compatible
    if requested_parts.len() == 2 {
        return true;
    }

    // If requested has patch version (e.g., "1.77.2"), it must match exactly
    if requested_parts.len() == 3 && installed_parts.len() >= 3 {
        return installed_parts[2] == requested_parts[2];
    }

    false
}

fn normalize_toolchain_for_rustup(toolchain: &str) -> String {
    // For nightly and beta - return as is
    if toolchain.starts_with("nightly-") || toolchain.starts_with("beta-") {
        return toolchain.to_string();
    }

    // For stable versions, ensure we have full version for rustup
    let parts: Vec<&str> = toolchain.split('.').collect();
    match parts.len() {
        2 => format!("{}.{}.0", parts[0], parts[1]), // 1.77 -> 1.77.0
        _ => toolchain.to_string(),                  // 1.77.2 -> 1.77.2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolchain_compatible_stable_versions() {
        // Test case: installed 1.87.0, requested 1.87 -> should be compatible
        assert!(toolchain_compatible("1.87.0", "1.87"));

        // Test case: installed 1.87.1, requested 1.87 -> should be compatible
        assert!(toolchain_compatible("1.87.1", "1.87"));

        // Test case: installed 1.87.2, requested 1.87 -> should be compatible
        assert!(toolchain_compatible("1.87.2", "1.87"));

        // Test case: installed 1.87.0, requested 1.87.0 -> should be compatible
        assert!(toolchain_compatible("1.87.0", "1.87.0"));

        // Test case: installed 1.87.1, requested 1.87.0 -> should NOT be compatible
        assert!(!toolchain_compatible("1.87.1", "1.87.0"));

        // Test case: installed 1.86.0, requested 1.87 -> should NOT be compatible
        assert!(!toolchain_compatible("1.86.0", "1.87"));

        // Test case: installed 1.87.0, requested 1.86 -> should NOT be compatible
        assert!(!toolchain_compatible("1.87.0", "1.86"));
    }

    #[test]
    fn test_toolchain_compatible_nightly_versions() {
        // Nightly must match exactly
        assert!(toolchain_compatible(
            "nightly-2024-06-01",
            "nightly-2024-06-01"
        ));
        assert!(!toolchain_compatible(
            "nightly-2024-06-01",
            "nightly-2024-06-02"
        ));

        // Mixed stable/nightly should not be compatible
        assert!(!toolchain_compatible("1.87.0", "nightly-2024-06-01"));
        assert!(!toolchain_compatible("nightly-2024-06-01", "1.87.0"));
    }

    #[test]
    fn test_toolchain_compatible_beta_versions() {
        // Beta must match exactly
        assert!(toolchain_compatible("beta-2024-05-15", "beta-2024-05-15"));
        assert!(!toolchain_compatible("beta-2024-05-15", "beta-2024-05-16"));

        // Mixed stable/beta should not be compatible
        assert!(!toolchain_compatible("1.87.0", "beta-2024-05-15"));
        assert!(!toolchain_compatible("beta-2024-05-15", "1.87.0"));
    }

    #[test]
    fn test_toolchain_compatible_edge_cases() {
        // Invalid versions
        assert!(!toolchain_compatible("1", "1.87"));
        assert!(!toolchain_compatible("1.87", "1.87.0"));

        // This is the case that's causing your issue!
        // Base image has 1.87.0, user requests 1.87
        assert!(toolchain_compatible("1.87.0", "1.87"));
    }

    #[test]
    fn test_normalize_toolchain_for_rustup() {
        assert_eq!(normalize_toolchain_for_rustup("1.87"), "1.87.0");
        assert_eq!(
            normalize_toolchain_for_rustup("nightly-2024-06-01"),
            "nightly-2024-06-01"
        );
    }
}

use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml::Value;

/// Validates Rust version (forbids generic channels)
pub fn validate_rust_version(version: &str) -> Result<()> {
    // Check for generic channels
    if matches!(version, "stable" | "nightly" | "beta") {
        bail!(
            "Generic channel '{}' is not allowed for reproducible builds.\n\
             Please specify an exact version like '1.86.0' or 'nightly-2024-01-01'.\n\
             \n\
             To set a specific version:\n\
             1. Update rust-toolchain.toml with a specific version\n\
             2. Or use --rust-version flag with exact version",
            version
        );
    }

    // Basic validation of version format
    if version.is_empty() {
        bail!("Rust version cannot be empty");
    }

    // Allow versions like "1.86.0", "1.86", "nightly-2024-01-01"
    let valid_chars = version
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '-');

    if !valid_chars {
        bail!(
            "Invalid Rust version format: '{}'. \
             Expected format like '1.86.0' or 'nightly-2024-01-01'",
            version
        );
    }

    Ok(())
}

/// Searches for Rust version in rust-toolchain.toml, walking up the directory tree
pub fn find_rust_toolchain_version(start_dir: &Path) -> Result<Option<String>> {
    let mut current_dir = start_dir;

    loop {
        // Check for rust-toolchain.toml
        let toolchain_toml = current_dir.join("rust-toolchain.toml");
        if toolchain_toml.exists() {
            match parse_rust_toolchain_toml(&toolchain_toml) {
                Ok(version) => return Ok(Some(version)),
                Err(e) => {
                    // Log warning but continue searching
                    eprintln!(
                        "Warning: Failed to parse {}: {}",
                        toolchain_toml.display(),
                        e
                    );
                }
            }
        }

        // Check for legacy rust-toolchain file
        let toolchain_file = current_dir.join("rust-toolchain");
        if toolchain_file.exists() {
            if let Ok(content) = fs::read_to_string(&toolchain_file) {
                let version = content.trim();
                if !version.is_empty() {
                    // Validate before returning
                    if let Err(e) = validate_rust_version(version) {
                        eprintln!(
                            "Warning: Invalid version in {}: {}",
                            toolchain_file.display(),
                            e
                        );
                    } else {
                        return Ok(Some(version.to_string()));
                    }
                }
            }
        }

        // Move to parent directory
        match current_dir.parent() {
            Some(parent) => current_dir = parent,
            None => break, // Reached root
        }
    }

    // Check RUSTUP_TOOLCHAIN environment variable as fallback
    if let Ok(toolchain) = std::env::var("RUSTUP_TOOLCHAIN") {
        // Extract version from toolchain string like "1.86.0-x86_64-unknown-linux-gnu"
        if let Some(version) = toolchain.split('-').next() {
            if validate_rust_version(version).is_ok() {
                return Ok(Some(version.to_string()));
            }
        }
    }

    Ok(None)
}

/// Parses rust-toolchain.toml file and extracts version
pub fn parse_rust_toolchain_toml(path: &Path) -> Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    // Try to parse as TOML
    let value: Value = toml::from_str(&content).with_context(|| {
        format!(
            "Failed to parse {} as TOML. Expected format:\n\
             [toolchain]\n\
             channel = \"1.86.0\"",
            path.display()
        )
    })?;

    // Extract channel from the toolchain section
    let channel = value
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Missing 'channel' in [toolchain] section of {}.\n\
                 Expected format:\n\
                 [toolchain]\n\
                 channel = \"1.86.0\"",
                path.display()
            )
        })?;

    // Validate the channel
    validate_rust_version(channel)?;

    Ok(channel.to_string())
}

#[allow(dead_code)]
/// Finds Cargo.toml by walking up the directory tree
pub fn find_cargo_toml(start_dir: &Path) -> Result<Option<PathBuf>> {
    let mut current_dir = start_dir;

    loop {
        let cargo_toml = current_dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return Ok(Some(cargo_toml));
        }

        // Move to parent directory
        match current_dir.parent() {
            Some(parent) => current_dir = parent,
            None => break, // Reached root
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_validate_rust_version() {
        // Valid versions
        assert!(validate_rust_version("1.86.0").is_ok());
        assert!(validate_rust_version("1.86").is_ok());
        assert!(validate_rust_version("nightly-2024-01-01").is_ok());

        // Invalid versions
        assert!(validate_rust_version("stable").is_err());
        assert!(validate_rust_version("nightly").is_err());
        assert!(validate_rust_version("beta").is_err());
        assert!(validate_rust_version("").is_err());
        assert!(validate_rust_version("1.86.0!@#").is_err());
    }

    #[test]
    fn test_parse_rust_toolchain_toml() -> Result<()> {
        let dir = tempdir()?;
        let toolchain_path = dir.path().join("rust-toolchain.toml");

        // Valid toolchain file
        let content = r#"
[toolchain]
channel = "1.86.0"
components = ["rustfmt", "clippy"]
"#;
        fs::write(&toolchain_path, content)?;
        assert_eq!(parse_rust_toolchain_toml(&toolchain_path)?, "1.86.0");

        // Invalid channel (generic)
        let content = r#"
[toolchain]
channel = "stable"
"#;
        fs::write(&toolchain_path, content)?;
        assert!(parse_rust_toolchain_toml(&toolchain_path).is_err());

        // Missing channel
        let content = r#"
[toolchain]
components = ["rustfmt"]
"#;
        fs::write(&toolchain_path, content)?;
        assert!(parse_rust_toolchain_toml(&toolchain_path).is_err());

        Ok(())
    }

    #[test]
    fn test_find_rust_toolchain_version() -> Result<()> {
        let dir = tempdir()?;
        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir)?;

        // Create toolchain file in parent
        let toolchain_path = dir.path().join("rust-toolchain.toml");
        let content = r#"
[toolchain]
channel = "1.85.0"
"#;
        fs::write(&toolchain_path, content)?;

        // Should find from subdirectory
        let version = find_rust_toolchain_version(&subdir)?;
        assert_eq!(version, Some("1.85.0".to_string()));

        Ok(())
    }
}

use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    env,
    path::{Path, PathBuf},
};

/// Simple registry for storing artifacts
pub struct ArtifactsRegistry {
    root_dir: PathBuf,
}

impl ArtifactsRegistry {
    /// Creates a new registry with the specified root directory
    /// If the provided path is relative, it will be relative to the current crate's directory
    /// (CARGO_MANIFEST_DIR). If an absolute path is provided, it will be used as is.
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        let path = root_dir.into();

        // If the path is absolute, use it directly
        // If it's relative, make it relative to CARGO_MANIFEST_DIR
        let root_dir = if path.is_absolute() {
            path
        } else {
            // Get the manifest directory of the current crate
            let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());

            PathBuf::from(manifest_dir).join(path)
        };

        Self { root_dir }
    }

    /// Stores serializable data to a file at the specified path
    pub fn store<T: Serialize>(&self, path: impl AsRef<Path>, data: &T) -> Result<PathBuf> {
        let full_path = self.root_dir.join(path);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create parent directories")?;
        }

        let content = serde_json::to_string_pretty(data).context("Failed to serialize data")?;

        std::fs::write(&full_path, content).context("Failed to write file")?;

        Ok(full_path)
    }

    /// Stores text content to a file at the specified path
    pub fn store_text(&self, path: impl AsRef<Path>, content: &str) -> Result<PathBuf> {
        let full_path = self.root_dir.join(path);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create parent directories")?;
        }

        std::fs::write(&full_path, content).context("Failed to write file")?;

        Ok(full_path)
    }

    /// Checks if a file exists at the specified path
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        self.root_dir.join(path).exists()
    }

    pub fn load<T: for<'de> serde::Deserialize<'de>>(&self, path: impl AsRef<Path>) -> Result<T> {
        let full_path = self.root_dir.join(path);

        let content = std::fs::read_to_string(&full_path).context(format!(
            "Failed to read file:{full_path:?}\nEnsure that you have added SolidityABI derivation for the struct"
        ))?;

        serde_json::from_str(&content).context("Failed to deserialize data")
    }
}

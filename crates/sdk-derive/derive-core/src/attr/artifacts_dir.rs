use darling::FromMeta;
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, PartialEq, Clone)]
pub struct Artifacts(pub PathBuf);

impl Artifacts {
    const DEFAULT_PATH: &'static str = "artifacts";

    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self(path.as_ref().to_path_buf())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Default for Artifacts {
    fn default() -> Self {
        Self::new(Self::DEFAULT_PATH)
    }
}

impl FromStr for Artifacts {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            Err("Artifacts path cannot be empty".to_string())
        } else {
            Ok(Self::new(s))
        }
    }
}

impl FromMeta for Artifacts {
    fn from_string(value: &str) -> darling::Result<Self> {
        value.parse().map_err(darling::Error::custom)
    }
}

impl fmt::Display for Artifacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

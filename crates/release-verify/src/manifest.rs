use crate::error::{Result, VerifyError};
use std::collections::BTreeMap;

/// The digest manifest published (and signed) alongside a release.
///
/// Format, as produced by `.github/workflows/release.yml`:
///
/// ```text
/// version=v1.3.2
/// commit=d6d8d2e739f50daa8174b299cb5170a9ce7e7974
///
/// [raw]
/// 4fcdd361…  ./crates/genesis/genesis-devnet.json
///
/// [compressed]
/// 92704da9…  ./artifacts/genesis-v1.3.2.json.gz
/// ```
///
/// Only the file name is significant, so a manifest stays usable regardless of the directory
/// layout the release job happened to run `sha256sum` from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseManifest {
    version: String,
    commit: String,
    digests: BTreeMap<String, [u8; 32]>,
}

impl ReleaseManifest {
    /// Parses a manifest. The caller must have authenticated `bytes` first.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|err| VerifyError::Manifest(format!("not valid UTF-8: {err}")))?;

        let mut version = None;
        let mut commit = None;
        let mut digests = BTreeMap::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            if let Some(value) = line.strip_prefix("version=") {
                if version.is_some() {
                    return Err(VerifyError::Manifest("multiple version= lines".to_owned()));
                }
                version = Some(value.to_owned());
                continue;
            }
            if let Some(value) = line.strip_prefix("commit=") {
                if commit.is_some() {
                    return Err(VerifyError::Manifest("multiple commit= lines".to_owned()));
                }
                commit = Some(value.to_owned());
                continue;
            }

            let (digest, path) = line
                .split_once(char::is_whitespace)
                .ok_or_else(|| VerifyError::Manifest(format!("unrecognised line {line:?}")))?;
            let digest: [u8; 32] = hex::decode(digest)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or_else(|| {
                    VerifyError::Manifest(format!("{line:?} does not start with a sha256"))
                })?;
            let name = path
                .trim()
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| VerifyError::Manifest(format!("{line:?} names no file")))?;

            if let Some(previous) = digests.insert(name.to_owned(), digest) {
                // Two different digests for one name would let a reader pick the convenient one.
                if previous != digest {
                    return Err(VerifyError::Manifest(format!(
                        "conflicting digests listed for {name}"
                    )));
                }
            }
        }

        Ok(Self {
            version: version.ok_or_else(|| VerifyError::Manifest("no version= line".to_owned()))?,
            commit: commit.ok_or_else(|| VerifyError::Manifest("no commit= line".to_owned()))?,
            digests,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn digest_for(&self, asset_name: &str) -> Option<[u8; 32]> {
        self.digests.get(asset_name).copied()
    }

    /// Requires that this manifest belongs to `tag` and vouches for `asset_name` at `sha256`.
    ///
    /// The version check is what stops a manifest from another release — or another network's
    /// release line — from being replayed against this artifact.
    pub fn check(&self, tag: &str, asset_name: &str, sha256: &[u8; 32]) -> Result<()> {
        if self.version != tag {
            return Err(VerifyError::Manifest(format!(
                "manifest is for release {}, expected {tag}",
                self.version
            )));
        }

        let expected = self
            .digest_for(asset_name)
            .ok_or_else(|| VerifyError::Manifest(format!("does not list {asset_name}")))?;

        if &expected != sha256 {
            return Err(VerifyError::DigestMismatch {
                name: asset_name.to_owned(),
                expected: hex::encode(expected),
                actual: hex::encode(sha256),
            });
        }

        Ok(())
    }
}

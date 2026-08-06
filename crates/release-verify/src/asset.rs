use std::borrow::Cow;

/// Where Fluent release assets are published.
pub const RELEASE_BASE_URL: &str = "https://github.com/fluentlabs-xyz/fluentbase/releases/download";

/// Byte budget for a compressed release artifact.
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

/// Byte budget for a detached signature.
pub const MAX_SIGNATURE_BYTES: usize = 64 * 1024;

/// Byte budget for a release manifest.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// Byte budget for decompressed genesis JSON.
pub const MAX_GENESIS_JSON_BYTES: u64 = 256 * 1024 * 1024;

/// A release asset to authenticate, identified by the release it belongs to and its file name.
///
/// The name is part of the identity on purpose: a signature that verifies over some other asset of
/// the same release must not be usable here, which is what the digest pin and the manifest lookup
/// (both keyed by name) enforce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    tag: Cow<'static, str>,
    name: Cow<'static, str>,
    sha256: Option<[u8; 32]>,
    max_bytes: usize,
}

impl ReleaseAsset {
    /// An arbitrary asset of release `tag`.
    pub fn new(tag: impl Into<Cow<'static, str>>, name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            tag: tag.into(),
            name: name.into(),
            sha256: None,
            max_bytes: MAX_ARTIFACT_BYTES,
        }
    }

    /// The compressed genesis asset of release `tag`, for the given channel (`None` = devnet).
    pub fn genesis(tag: impl Into<Cow<'static, str>>, channel: Option<&str>) -> Self {
        let tag = tag.into();
        let name = match channel {
            Some(channel) => format!("genesis-{channel}-{tag}.json.gz"),
            None => format!("genesis-{tag}.json.gz"),
        };
        Self::new(tag, name)
    }

    /// The signed digest manifest of release `tag`.
    pub fn manifest(tag: impl Into<Cow<'static, str>>) -> Self {
        let tag = tag.into();
        let name = format!("genesis-manifest-{tag}.txt");
        Self::new(tag, name).with_max_bytes(MAX_MANIFEST_BYTES)
    }

    /// Pins the exact SHA-256 this asset must have.
    pub fn with_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.sha256 = Some(sha256);
        self
    }

    /// Overrides the byte budget for this asset.
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sha256(&self) -> Option<[u8; 32]> {
        self.sha256
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Name of the detached signature that accompanies this asset.
    pub fn signature_name(&self) -> String {
        format!("{}.asc", self.name)
    }

    pub fn url(&self) -> String {
        format!("{RELEASE_BASE_URL}/{}/{}", self.tag, self.name)
    }

    pub fn signature_url(&self) -> String {
        format!("{}.asc", self.url())
    }
}

use std::fmt::Display;

/// Everything that can stop an artifact from being trusted.
///
/// Every variant is a refusal: there is no "warning" outcome and no bypass. Callers turn this into
/// their own error type (`anyhow`, `eyre`, …) at the boundary.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("failed to parse the release public key: {0}")]
    KeyParse(String),

    #[error("release public key has invalid self-signatures: {0}")]
    KeyBindings(String),

    #[error("release public key fingerprint mismatch: expected {expected}, got {actual}")]
    KeyFingerprint { expected: String, actual: String },

    #[error("failed to parse detached signature: {0}")]
    SignatureParse(String),

    #[error("unexpected signature type {0}, expected a binary signature")]
    SignatureType(String),

    #[error("signature uses rejected digest algorithm {0}")]
    WeakDigest(String),

    #[error("signature does not identify an issuer")]
    MissingIssuer,

    #[error("signature was not issued by the pinned release key")]
    UntrustedIssuer,

    #[error("signature does not verify against the release key: {0}")]
    BadSignature(String),

    #[error("digest mismatch for {name}: expected sha256 {expected}, got {actual}")]
    DigestMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("{what} exceeds the {limit} byte limit")]
    TooLarge { what: String, limit: usize },

    #[error("release manifest: {0}")]
    Manifest(String),

    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to fetch {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: FetchError,
    },

    #[error("failed to decompress {0}")]
    Decompress(String),

    #[error("failed to parse genesis JSON: {0}")]
    GenesisParse(String),
}

/// Transport-level failure reported by a [`crate::Fetcher`].
///
/// Deliberately a flat string: it exists so callers can plug in any HTTP client without this crate
/// having to agree with them on an error type.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct FetchError(String);

impl FetchError {
    pub fn new(error: impl Display) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, VerifyError>;

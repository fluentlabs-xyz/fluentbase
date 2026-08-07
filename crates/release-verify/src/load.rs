use crate::{
    asset::{ReleaseAsset, MAX_GENESIS_JSON_BYTES, MAX_SIGNATURE_BYTES},
    error::{FetchError, Result, VerifyError},
    key::ReleaseKey,
    signature::verify_detached_signature,
};
use alloy_genesis::Genesis;
use sha2::{Digest as _, Sha256};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tracing::warn;

/// Fetches `url` into memory, refusing anything larger than the given byte budget.
///
/// Callers supply their own HTTP client; this crate never opens a socket.
pub type Fetcher<'a> = dyn Fn(&str, usize) -> std::result::Result<Vec<u8>, FetchError> + 'a;

/// An artifact that passed authentication, together with the digest it was authenticated at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifact {
    /// File name of the asset, as published.
    pub name: String,
    /// The exact bytes that were verified. Anything derived from the artifact must come from here.
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes`.
    pub sha256: [u8; 32],
}

impl VerifiedArtifact {
    /// Hex digest, for logging and for manifest cross-checks.
    pub fn sha256_hex(&self) -> String {
        hex::encode(self.sha256)
    }
}

/// Loads a release asset from cache or from the release, authenticating it before it is usable.
///
/// The order of operations is the security property: bytes are held in memory, checked against the
/// digest pin (when there is one) and the detached signature, and only then returned or written to
/// the cache. A cached pair that fails authentication is deleted and re-fetched; it is never used.
/// If the asset cannot be authenticated, this returns an error — there is no degraded mode.
pub fn load_verified(
    cache_dir: Option<&Path>,
    asset: &ReleaseAsset,
    key: &ReleaseKey,
    fetch: &Fetcher<'_>,
) -> Result<VerifiedArtifact> {
    let cached_paths = cache_dir.map(|dir| {
        (
            dir.join(asset.name()),
            dir.join(asset.signature_name()),
            dir,
        )
    });

    if let Some((artifact_path, signature_path, _)) = &cached_paths {
        if let Some((bytes, asc)) = read_cached_pair(artifact_path, signature_path, asset) {
            match authenticate(&bytes, &asc, asset, key) {
                Ok(artifact) => return Ok(artifact),
                Err(err) => {
                    // Never fall back to the cached bytes: drop them and re-fetch.
                    warn!(
                        "cached {} failed authentication ({err}); discarding and re-downloading",
                        artifact_path.display()
                    );
                    let _ = fs::remove_file(artifact_path);
                    let _ = fs::remove_file(signature_path);
                }
            }
        }
    }

    let url = asset.url();
    let bytes = fetch(&url, asset.max_bytes()).map_err(|source| VerifyError::Fetch {
        url: url.clone(),
        source,
    })?;
    let signature_url = asset.signature_url();
    let asc = fetch(&signature_url, MAX_SIGNATURE_BYTES).map_err(|source| VerifyError::Fetch {
        url: signature_url,
        source,
    })?;

    let artifact = authenticate(&bytes, &asc, asset, key)?;

    // The cache only ever receives material that already authenticated, so a later run cannot be
    // handed bytes this run would have rejected. Caching is best effort.
    if let Some((artifact_path, signature_path, dir)) = &cached_paths {
        if let Err(err) = cache_pair(dir, artifact_path, &bytes, signature_path, &asc) {
            warn!("failed to cache verified {}: {err}", asset.name());
        }
    }

    Ok(artifact)
}

/// Authenticates in-memory artifact bytes: digest pin first (when known), then signature.
pub fn authenticate(
    bytes: &[u8],
    armored_sig: &[u8],
    asset: &ReleaseAsset,
    key: &ReleaseKey,
) -> Result<VerifiedArtifact> {
    let sha256: [u8; 32] = Sha256::digest(bytes).into();

    if let Some(expected) = asset.sha256() {
        if sha256 != expected {
            return Err(VerifyError::DigestMismatch {
                name: asset.name().to_owned(),
                expected: hex::encode(expected),
                actual: hex::encode(sha256),
            });
        }
    }

    verify_detached_signature(bytes, armored_sig, key)?;

    Ok(VerifiedArtifact {
        name: asset.name().to_owned(),
        bytes: bytes.to_vec(),
        sha256,
    })
}

/// Decompresses and parses authenticated genesis bytes.
///
/// Takes a [`VerifiedArtifact`] rather than a byte slice so that unauthenticated input cannot reach
/// the parser by accident.
pub fn parse_genesis_gz(artifact: &VerifiedArtifact) -> Result<Genesis> {
    let json = decompress_gz(&artifact.bytes, &artifact.name)?;
    serde_json::from_slice::<Genesis>(&json)
        .map_err(|err| VerifyError::GenesisParse(err.to_string()))
}

/// Decompresses authenticated gzip bytes under the JSON size cap.
pub fn decompress_gz(bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .take(MAX_GENESIS_JSON_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|err| VerifyError::Decompress(format!("{what}: {err}")))?;
    if out.len() as u64 > MAX_GENESIS_JSON_BYTES {
        return Err(VerifyError::TooLarge {
            what: format!("decompressed {what}"),
            limit: MAX_GENESIS_JSON_BYTES as usize,
        });
    }
    Ok(out)
}

/// Reads a cached artifact / signature pair, or `None` when either side is missing or unreadable.
fn read_cached_pair(
    artifact_path: &Path,
    signature_path: &Path,
    asset: &ReleaseAsset,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let bytes = read_capped(artifact_path, asset.max_bytes()).ok()?;
    let asc = read_capped(signature_path, MAX_SIGNATURE_BYTES).ok()?;
    Some((bytes, asc))
}

/// Reads at most `max_bytes` from `path`, failing if the file is larger.
pub fn read_capped(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|source| VerifyError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|source| VerifyError::Io {
            path: path.display().to_string(),
            source,
        })?;
    if buf.len() > max_bytes {
        return Err(VerifyError::TooLarge {
            what: path.display().to_string(),
            limit: max_bytes,
        });
    }
    Ok(buf)
}

fn cache_pair(
    cache_dir: &Path,
    artifact_path: &Path,
    bytes: &[u8],
    signature_path: &Path,
    asc: &[u8],
) -> Result<()> {
    fs::create_dir_all(cache_dir).map_err(|source| VerifyError::Io {
        path: cache_dir.display().to_string(),
        source,
    })?;
    write_atomic(artifact_path, bytes)?;
    write_atomic(signature_path, asc)
}

/// Writes `bytes` to `path` atomically (write to temp, then rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let io_err = |path: &Path| {
        let path = path.display().to_string();
        move |source| VerifyError::Io {
            path: path.clone(),
            source,
        }
    };

    let (tmp, mut file) = loop {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let mut tmp_name = OsString::from(path.as_os_str());
        tmp_name.push(format!(".{}.{}.tmp", std::process::id(), id));
        let tmp = PathBuf::from(tmp_name);
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_err(&tmp)(source)),
        }
    };

    let write_result = (|| {
        file.write_all(bytes).map_err(io_err(&tmp))?;
        file.sync_all().map_err(io_err(&tmp))?;
        drop(file);
        fs::rename(&tmp, path).map_err(io_err(path))
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

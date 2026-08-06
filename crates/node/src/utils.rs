//! Authenticated retrieval of the genesis artifacts that back the built-in networks.
//!
//! Built-in networks (devnet / testnet / mainnet) take their genesis state from a `.json.gz`
//! asset published on GitHub releases, together with a detached OpenPGP signature (`.asc`).
//! Because that state defines allocations and system contract code, the artifact is treated as
//! untrusted input until it has been authenticated against the release key pinned in this file.
//!
//! The rules enforced here are deliberately fail-closed:
//!
//! * the artifact is only ever held in memory until authentication succeeds — nothing is
//!   decompressed, parsed, or written to the cache before that;
//! * the signature is checked over the exact compressed bytes that are later decompressed, so
//!   there is no window between verification and use;
//! * cached files get the same treatment as freshly downloaded ones;
//! * where a digest pin is known it is checked in addition to the signature;
//! * every read, parse, signer or signature error aborts startup. There is no bypass switch.

use alloy_genesis::Genesis;
use eyre::{bail, eyre, WrapErr as _};
use pgp::{
    composed::{Deserializable as _, DetachedSignature, SignedPublicKey, SignedPublicSubKey},
    crypto::hash::HashAlgorithm,
    packet::SignatureType,
    types::{Fingerprint, KeyDetails as _, KeyId},
};
use sha2::{Digest as _, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::warn;

/// Upper bound on the compressed artifact we are willing to hold in memory.
const MAX_GENESIS_GZ_BYTES: usize = 64 * 1024 * 1024;

/// Upper bound on a detached signature file.
const MAX_SIGNATURE_BYTES: usize = 64 * 1024;

/// Upper bound on the decompressed genesis JSON.
const MAX_GENESIS_JSON_BYTES: u64 = 256 * 1024 * 1024;

/// Timeout for a single artifact download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// ASCII-armored OpenPGP public key used to authenticate genesis artifacts.
const FLUENT_RELEASE_PUBKEY_ASC: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEaEq6ORYJKwYBBAHaRw8BAQdADSciIyJRuaPogw2vJ388jlOsKRQk1c84vUpn
NT+vmeu0J0RtaXRyaWkgU2F2b25pbiA8ZG1pdHJ5QGZsdWVudGxhYnMueHl6PoiT
BBMWCgA7FiEECm0F5d2YBpuhhO2DBKaNYg1SCP0FAmhKujkCGwMFCwkIBwICIgIG
FQoJCAsCBBYCAwECHgcCF4AACgkQBKaNYg1SCP0eRwEA43IlexWb2Nh/rVzVyRVg
fPLZ45a13AP0iMCnAhjFK/cBAL5zDzWNNFkxHm6XGYQC4mHWLeZFe3gIJVQ0Y+wH
hCoHuDgEaEq6ORIKKwYBBAGXVQEFAQEHQCBTP3PIjJhuMZdF5aVuEiPODt9EpEnK
Jph+AW0cmfZ2AwEIB4h4BBgWCgAgFiEECm0F5d2YBpuhhO2DBKaNYg1SCP0FAmhK
ujkCGwwACgkQBKaNYg1SCP2KwgD/UJk7eQhlLNosZNLOyFj48241KcG2lJbCgzt8
XehpkCgA/13esUBYao//zRco9fgrVbSBNJ7FO1G0jXAYygDqCYsJ
=Ortc
-----END PGP PUBLIC KEY BLOCK-----";

/// Fingerprint the embedded release certificate must have.
///
/// Pinning the fingerprint separately from the armored blob means a subtle edit to the key
/// material above cannot go unnoticed: [`load_release_cert`] refuses to build a verifier from a
/// certificate whose fingerprint does not match this constant.
const FLUENT_RELEASE_KEY_FINGERPRINT: &str = "0A6D05E5DD98069BA184ED8304A68D620D5208FD";

/// Digest algorithms accepted on a genesis signature. SHA-1 and friends are rejected outright.
const ACCEPTED_HASH_ALGORITHMS: &[HashAlgorithm] = &[
    HashAlgorithm::Sha256,
    HashAlgorithm::Sha384,
    HashAlgorithm::Sha512,
    HashAlgorithm::Sha3_256,
    HashAlgorithm::Sha3_512,
];

/// A genesis artifact published for a built-in network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenesisArtifact {
    /// Release tag the artifact was published under.
    pub tag: &'static str,
    /// Release channel, part of the asset name when present.
    pub channel: Option<&'static str>,
    /// SHA-256 of the exact `.json.gz` asset, when a pin is available for this release.
    pub sha256: Option<[u8; 32]>,
}

impl GenesisArtifact {
    /// Name of the compressed genesis asset.
    pub fn gz_name(&self) -> String {
        match self.channel {
            Some(channel) => format!("genesis-{channel}-{}.json.gz", self.tag),
            None => format!("genesis-{}.json.gz", self.tag),
        }
    }

    /// Name of the detached signature asset.
    pub fn asc_name(&self) -> String {
        format!("{}.asc", self.gz_name())
    }

    /// Download URL of the compressed genesis asset.
    pub fn gz_url(&self) -> String {
        format!("{}/{}", self.release_base_url(), self.gz_name())
    }

    /// Download URL of the detached signature asset.
    pub fn asc_url(&self) -> String {
        format!("{}/{}", self.release_base_url(), self.asc_name())
    }

    fn release_base_url(&self) -> String {
        format!(
            "https://github.com/fluentlabs-xyz/fluentbase/releases/download/{}",
            self.tag
        )
    }
}

/// Fetches `url` into memory, refusing anything larger than the given byte budget.
type Fetcher<'a> = dyn Fn(&str, usize) -> eyre::Result<Vec<u8>> + 'a;

/// Loads the genesis of a built-in network, authenticating the artifact before use.
///
/// This is intentionally synchronous because it runs during CLI startup / chainspec selection.
pub fn download_and_cache_genesis_verified(artifact: &GenesisArtifact) -> eyre::Result<Genesis> {
    let cache_dir = genesis_cache_dir()?;
    load_genesis_verified(
        &cache_dir,
        artifact,
        FLUENT_RELEASE_PUBKEY_ASC,
        FLUENT_RELEASE_KEY_FINGERPRINT,
        &http_fetch,
    )
}

/// Cache-aware, fail-closed genesis loading.
///
/// Split out from [`download_and_cache_genesis_verified`] so tests can drive it with a throwaway
/// release key, a fake fetcher, and a throwaway cache directory.
fn load_genesis_verified(
    cache_dir: &Path,
    artifact: &GenesisArtifact,
    pubkey_asc: &str,
    expected_fingerprint: &str,
    fetch: &Fetcher<'_>,
) -> eyre::Result<Genesis> {
    // A key that does not load, or does not match its pin, aborts startup — it is never a reason
    // to skip verification.
    let cert = load_release_cert(pubkey_asc, expected_fingerprint)?;

    let gz_path = cache_dir.join(artifact.gz_name());
    let asc_path = cache_dir.join(artifact.asc_name());

    println!("Checking genesis for tag {}...", artifact.tag);

    // Fast path: a cached pair that authenticates against the pinned key.
    if let Some((gz, asc)) = read_cached_pair(&gz_path, &asc_path) {
        match authenticate(&gz, &asc, artifact, &cert) {
            Ok(()) => {
                println!("Using cached genesis from {}", gz_path.display());
                return parse_genesis_gz(&gz);
            }
            Err(err) => {
                // Never fall back to the cached bytes: drop them and re-fetch from the release.
                warn!(
                    "cached genesis {} failed authentication ({err:#}); discarding and re-downloading",
                    gz_path.display()
                );
                let _ = fs::remove_file(&gz_path);
                let _ = fs::remove_file(&asc_path);
            }
        }
    }

    let gz_url = artifact.gz_url();
    println!("Genesis not available from cache, downloading from {gz_url}");

    let gz = fetch(&gz_url, MAX_GENESIS_GZ_BYTES).wrap_err("failed to download genesis .gz")?;
    let asc = fetch(&artifact.asc_url(), MAX_SIGNATURE_BYTES)
        .wrap_err("failed to download genesis .asc")?;

    println!("Verifying genesis signature...");
    authenticate(&gz, &asc, artifact, &cert).wrap_err_with(|| {
        format!(
            "genesis authentication failed for {} — refusing to start",
            artifact.gz_name()
        )
    })?;

    // The cache is only populated with material that already authenticated, so a later run can
    // never be handed bytes this run would have rejected. Caching is best effort.
    if let Err(err) = cache_artifact(cache_dir, &gz_path, &gz, &asc_path, &asc) {
        warn!("failed to cache verified genesis artifact: {err:#}");
    }

    parse_genesis_gz(&gz)
}

/// Parses an armored release certificate and checks it against its pinned fingerprint.
fn load_release_cert(
    pubkey_asc: &str,
    expected_fingerprint: &str,
) -> eyre::Result<SignedPublicKey> {
    let (cert, _headers) = SignedPublicKey::from_string(pubkey_asc)
        .map_err(|err| eyre!("failed to parse the release public key: {err}"))?;

    cert.verify_bindings()
        .map_err(|err| eyre!("release public key has invalid self-signatures: {err}"))?;

    let fingerprint = fingerprint_hex(&cert.fingerprint());
    if fingerprint != expected_fingerprint {
        bail!(
            "release public key fingerprint mismatch: expected {}, got {}",
            expected_fingerprint,
            fingerprint
        );
    }

    Ok(cert)
}

/// Authenticates the compressed artifact: digest pin first (when known), then signature.
fn authenticate(
    gz: &[u8],
    asc: &[u8],
    artifact: &GenesisArtifact,
    cert: &SignedPublicKey,
) -> eyre::Result<()> {
    if let Some(expected) = artifact.sha256 {
        let actual: [u8; 32] = Sha256::digest(gz).into();
        if actual != expected {
            bail!(
                "genesis artifact digest mismatch for {}: expected sha256 {}, got {}",
                artifact.gz_name(),
                hex_lower(&expected),
                hex_lower(&actual)
            );
        }
    }

    verify_detached_signature(gz, asc, cert)
}

/// Verifies a detached OpenPGP signature over `data` against the pinned release certificate.
///
/// Beyond the cryptographic check this rejects signatures that are structurally unusable: text
/// mode signatures (which do not bind the exact bytes), weak digests, and signatures whose issuer
/// is not a signing-capable component of the pinned certificate.
fn verify_detached_signature(
    data: &[u8],
    armored_sig: &[u8],
    cert: &SignedPublicKey,
) -> eyre::Result<()> {
    let (detached, _headers) = DetachedSignature::from_armor_single(Cursor::new(armored_sig))
        .map_err(|err| eyre!("failed to parse detached signature: {err}"))?;
    let sig = &detached.signature;

    match sig.typ() {
        Some(SignatureType::Binary) => {}
        Some(other) => bail!("unexpected signature type {other:?}, expected a binary signature"),
        None => bail!("signature is of an unknown type"),
    }

    let hash_alg = sig
        .hash_alg()
        .ok_or_else(|| eyre!("signature does not declare a digest algorithm"))?;
    if !ACCEPTED_HASH_ALGORITHMS.contains(&hash_alg) {
        bail!("signature uses rejected digest algorithm {hash_alg:?}");
    }

    let issuer_fingerprints: Vec<&Fingerprint> = sig.issuer_fingerprint();
    let issuer_key_ids: Vec<&KeyId> = sig.issuer_key_id();
    if issuer_fingerprints.is_empty() && issuer_key_ids.is_empty() {
        bail!("signature does not identify an issuer");
    }

    for candidate in signing_candidates(cert) {
        let is_issuer = issuer_fingerprints
            .iter()
            .any(|fpr| **fpr == candidate.fingerprint())
            || issuer_key_ids
                .iter()
                .any(|id| **id == candidate.legacy_key_id());
        if !is_issuer {
            continue;
        }
        return candidate
            .verify(&detached, data)
            .map_err(|err| eyre!("signature does not verify against the release key: {err}"));
    }

    bail!("signature was not issued by the pinned release key")
}

/// A component of the release certificate that may have produced a data signature.
enum SigningCandidate<'a> {
    Primary(&'a pgp::packet::PublicKey),
    Subkey(&'a SignedPublicSubKey),
}

impl SigningCandidate<'_> {
    fn fingerprint(&self) -> Fingerprint {
        match self {
            Self::Primary(key) => key.fingerprint(),
            Self::Subkey(key) => key.fingerprint(),
        }
    }

    fn legacy_key_id(&self) -> KeyId {
        match self {
            Self::Primary(key) => key.legacy_key_id(),
            Self::Subkey(key) => key.legacy_key_id(),
        }
    }

    fn verify(&self, sig: &DetachedSignature, data: &[u8]) -> pgp::errors::Result<()> {
        match self {
            Self::Primary(key) => sig.verify(*key, data),
            Self::Subkey(key) => sig.verify(*key, data),
        }
    }
}

/// The primary key plus every signing-capable subkey of `cert`.
fn signing_candidates(cert: &SignedPublicKey) -> Vec<SigningCandidate<'_>> {
    let mut candidates = vec![SigningCandidate::Primary(&cert.primary_key)];
    candidates.extend(
        cert.public_subkeys
            .iter()
            .filter(|subkey| subkey.signatures.iter().any(|sig| sig.key_flags().sign()))
            .map(SigningCandidate::Subkey),
    );
    candidates
}

/// Reads a cached `.gz` / `.asc` pair, or `None` when either side is missing or unreadable.
fn read_cached_pair(gz_path: &Path, asc_path: &Path) -> Option<(Vec<u8>, Vec<u8>)> {
    let gz = read_capped(gz_path, MAX_GENESIS_GZ_BYTES).ok()?;
    let asc = read_capped(asc_path, MAX_SIGNATURE_BYTES).ok()?;
    Some((gz, asc))
}

/// Writes both artifacts to the cache atomically.
fn cache_artifact(
    cache_dir: &Path,
    gz_path: &Path,
    gz: &[u8],
    asc_path: &Path,
    asc: &[u8],
) -> eyre::Result<()> {
    fs::create_dir_all(cache_dir)
        .wrap_err_with(|| format!("failed to create genesis cache dir {}", cache_dir.display()))?;
    write_atomic(gz_path, gz)?;
    write_atomic(asc_path, asc)
}

/// Where to cache genesis files.
fn genesis_cache_dir() -> eyre::Result<PathBuf> {
    let proj = directories::ProjectDirs::from("xyz", "fluentlabs", "fluent")
        .ok_or_else(|| eyre::eyre!("cannot determine cache directory"))?;
    Ok(proj.cache_dir().join("genesis"))
}

/// Downloads `url` into memory, refusing responses larger than `max_bytes`.
fn http_fetch(url: &str, max_bytes: usize) -> eyre::Result<Vec<u8>> {
    // NOTE: blocking client avoids pulling tokio into a CLI dependency tree.
    let resp = reqwest::blocking::Client::builder()
        .user_agent("fluent-chainspec/1.0")
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .wrap_err("failed to build HTTP client")?
        .get(url)
        .send()
        .wrap_err_with(|| format!("GET {url}"))?
        .error_for_status()
        .wrap_err_with(|| format!("GET {url} returned non-success"))?;

    if let Some(len) = resp.content_length() {
        if len > max_bytes as u64 {
            bail!("{url} advertises {len} bytes, over the {max_bytes} byte limit");
        }
    }

    let mut buf = Vec::new();
    resp.take(max_bytes as u64 + 1)
        .read_to_end(&mut buf)
        .wrap_err_with(|| format!("reading body from {url}"))?;
    if buf.len() > max_bytes {
        bail!("{url} exceeds the {max_bytes} byte limit");
    }
    Ok(buf)
}

/// Reads at most `max_bytes` from `path`, failing if the file is larger.
fn read_capped(path: &Path, max_bytes: usize) -> eyre::Result<Vec<u8>> {
    let file =
        fs::File::open(path).wrap_err_with(|| format!("failed to open {}", path.display()))?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut buf)
        .wrap_err_with(|| format!("failed to read {}", path.display()))?;
    if buf.len() > max_bytes {
        bail!("{} exceeds the {max_bytes} byte limit", path.display());
    }
    Ok(buf)
}

/// Writes `bytes` to `path` atomically (write to temp, then rename).
fn write_atomic(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let mut tmp_name = OsString::from(path.as_os_str());
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);

    {
        let mut f = fs::File::create(&tmp)
            .wrap_err_with(|| format!("failed to create {}", tmp.display()))?;
        f.write_all(bytes)
            .wrap_err_with(|| format!("failed to write {}", tmp.display()))?;
        f.sync_all()
            .wrap_err_with(|| format!("failed to sync {}", tmp.display()))?;
    }

    fs::rename(&tmp, path)
        .wrap_err_with(|| format!("failed to move {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Decompresses and parses authenticated genesis bytes.
///
/// Callers must only reach this with material that passed [`authenticate`].
fn parse_genesis_gz(gz: &[u8]) -> eyre::Result<Genesis> {
    let decoder = flate2::read::GzDecoder::new(gz);
    let mut json = Vec::new();
    decoder
        .take(MAX_GENESIS_JSON_BYTES + 1)
        .read_to_end(&mut json)
        .wrap_err("failed to decompress genesis gz")?;
    if json.len() as u64 > MAX_GENESIS_JSON_BYTES {
        bail!(
            "genesis JSON exceeds {} byte decompressed limit",
            MAX_GENESIS_JSON_BYTES
        );
    }
    serde_json::from_slice::<Genesis>(&json).wrap_err("failed to parse genesis JSON")
}

fn fingerprint_hex(fingerprint: &Fingerprint) -> String {
    fingerprint
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chainspec::BUILT_IN_GENESIS_ARTIFACTS;
    use flate2::{write::GzEncoder, Compression};
    use pgp::{
        composed::{KeyType, SecretKeyParamsBuilder, SignedSecretKey},
        packet::Signature,
        types::Password,
    };
    use std::cell::RefCell;

    /// Gzips `json` the way a release artifact is packed.
    fn gzip(json: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(json).unwrap();
        encoder.finish().unwrap()
    }

    /// Minimal but valid genesis JSON, gzipped, used as a stand-in for a release artifact.
    fn sample_genesis_gz() -> Vec<u8> {
        gzip(br#"{"config":{"chainId":1337},"alloc":{},"gasLimit":"0x1c9c380","difficulty":"0x0"}"#)
    }

    /// A genesis an attacker would like the node to boot from instead.
    fn substituted_genesis_gz() -> Vec<u8> {
        gzip(br#"{"config":{"chainId":31337},"alloc":{},"gasLimit":"0x1","difficulty":"0x0"}"#)
    }

    /// A throwaway release identity: secret key, armored certificate, and its fingerprint pin.
    struct TestKey {
        secret: SignedSecretKey,
        armored: String,
        fingerprint: String,
    }

    impl TestKey {
        fn new(user_id: &str) -> Self {
            let params = SecretKeyParamsBuilder::default()
                .key_type(KeyType::Ed25519Legacy)
                .can_certify(true)
                .can_sign(true)
                .primary_user_id(user_id.into())
                .build()
                .expect("key params");
            let secret = params.generate(rand_08::rngs::OsRng).expect("generate key");
            let public = secret.to_public_key();
            let armored = public
                .to_armored_string(Default::default())
                .expect("armor public key");
            let fingerprint = fingerprint_hex(&public.fingerprint());
            Self {
                secret,
                armored,
                fingerprint,
            }
        }

        fn cert(&self) -> SignedPublicKey {
            load_release_cert(&self.armored, &self.fingerprint).expect("test cert must load")
        }

        fn sign(&self, data: &[u8]) -> Vec<u8> {
            self.sign_with_hash(data, HashAlgorithm::Sha256)
        }

        fn sign_with_hash(&self, data: &[u8], hash: HashAlgorithm) -> Vec<u8> {
            armor(
                DetachedSignature::sign_binary_data(
                    rand_08::rngs::OsRng,
                    &self.secret.primary_key,
                    &Password::empty(),
                    hash,
                    data,
                )
                .expect("sign"),
            )
        }

        fn sign_text(&self, data: &[u8]) -> Vec<u8> {
            armor(
                DetachedSignature::sign_text_data(
                    rand_08::rngs::OsRng,
                    &self.secret.primary_key,
                    &Password::empty(),
                    HashAlgorithm::Sha256,
                    data,
                )
                .expect("sign"),
            )
        }
    }

    fn armor(sig: DetachedSignature) -> Vec<u8> {
        sig.to_armored_bytes(Default::default())
            .expect("armor signature")
    }

    fn parse_sig(asc: &[u8]) -> DetachedSignature {
        DetachedSignature::from_armor_single(Cursor::new(asc))
            .expect("parse signature")
            .0
    }

    fn artifact() -> GenesisArtifact {
        GenesisArtifact {
            tag: "v9.9.9",
            channel: None,
            sha256: None,
        }
    }

    /// Fetcher that serves a release's two assets and records how often it was called.
    struct FakeRelease {
        gz: Vec<u8>,
        asc: Vec<u8>,
        artifact: GenesisArtifact,
        calls: RefCell<usize>,
    }

    impl FakeRelease {
        fn new(artifact: GenesisArtifact, gz: Vec<u8>, asc: Vec<u8>) -> Self {
            Self {
                gz,
                asc,
                artifact,
                calls: RefCell::new(0),
            }
        }

        fn fetch(&self, url: &str, max_bytes: usize) -> eyre::Result<Vec<u8>> {
            *self.calls.borrow_mut() += 1;
            let body = if url == self.artifact.gz_url() {
                self.gz.clone()
            } else if url == self.artifact.asc_url() {
                self.asc.clone()
            } else {
                bail!("404 for {url}");
            };
            if body.len() > max_bytes {
                bail!("{url} exceeds the {max_bytes} byte limit");
            }
            Ok(body)
        }

        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }

    /// Plants an artifact pair in `dir` as if it had been cached by an earlier run.
    fn plant_cache(dir: &Path, artifact: &GenesisArtifact, gz: &[u8], asc: &[u8]) {
        fs::write(dir.join(artifact.gz_name()), gz).unwrap();
        fs::write(dir.join(artifact.asc_name()), asc).unwrap();
    }

    // ---------------------------------------------------------------------
    // The pinned release key
    // ---------------------------------------------------------------------

    #[test]
    fn embedded_release_key_parses_and_matches_its_pin() {
        let cert = load_release_cert(FLUENT_RELEASE_PUBKEY_ASC, FLUENT_RELEASE_KEY_FINGERPRINT)
            .expect("embedded key must load");
        assert_eq!(
            fingerprint_hex(&cert.fingerprint()),
            FLUENT_RELEASE_KEY_FINGERPRINT
        );
        assert!(
            !signing_candidates(&cert).is_empty(),
            "embedded key must expose at least one signing component"
        );
    }

    #[test]
    fn release_key_that_misses_its_fingerprint_pin_is_rejected() {
        let impostor = TestKey::new("Impostor <impostor@example.com>");
        let err = load_release_cert(&impostor.armored, FLUENT_RELEASE_KEY_FINGERPRINT)
            .expect_err("must reject a key that does not match the pin");
        assert!(
            err.to_string().contains("fingerprint mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn malformed_release_key_is_rejected() {
        let _ = load_release_cert("not a pgp key at all", FLUENT_RELEASE_KEY_FINGERPRINT)
            .expect_err("must reject garbage key");
    }

    #[test]
    fn startup_fails_closed_when_the_release_key_does_not_match_its_pin() {
        let release = TestKey::new("Release <release@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();
        let gz = sample_genesis_gz();
        let release_feed = FakeRelease::new(artifact, gz.clone(), release.sign(&gz));

        let err = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            FLUENT_RELEASE_KEY_FINGERPRINT,
            &|url, max| release_feed.fetch(url, max),
        )
        .expect_err("a key that misses its pin must abort startup");
        assert!(
            err.to_string().contains("fingerprint mismatch"),
            "unexpected error: {err}"
        );
        assert_eq!(release_feed.calls(), 0, "nothing should have been fetched");
    }

    // ---------------------------------------------------------------------
    // Signature verification
    // ---------------------------------------------------------------------

    #[test]
    fn valid_signature_is_accepted() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let sig = release.sign(&data);

        verify_detached_signature(&data, &sig, &release.cert())
            .expect("valid signature must verify");
    }

    #[test]
    fn signature_over_different_content_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let sig = release.sign(&data);

        let mut tampered = data.clone();
        *tampered.last_mut().unwrap() ^= 0x01;

        let err = verify_detached_signature(&tampered, &sig, &release.cert())
            .expect_err("content mismatch must be rejected");
        assert!(
            err.to_string().contains("does not verify"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn signature_from_a_different_key_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let data = sample_genesis_gz();
        let sig = attacker.sign(&data);

        let err = verify_detached_signature(&data, &sig, &release.cert())
            .expect_err("wrong-key signature must be rejected");
        assert!(
            err.to_string()
                .contains("not issued by the pinned release key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn signature_with_a_spoofed_issuer_is_rejected() {
        // Issuer subpackets are attacker-controlled hints; only the cryptographic check counts.
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let data = sample_genesis_gz();

        let mut forged = parse_sig(&attacker.sign(&data));
        let genuine = parse_sig(&release.sign(&data));
        let genuine_issuers = genuine
            .signature
            .config()
            .expect("config")
            .unhashed_subpackets
            .clone();
        for (idx, subpacket) in genuine_issuers.into_iter().enumerate() {
            forged
                .signature
                .unhashed_subpacket_insert(idx, subpacket)
                .expect("insert issuer subpacket");
        }

        let err = verify_detached_signature(&data, &armor(forged), &release.cert())
            .expect_err("a forged issuer must not make a bad signature verify");
        assert!(
            err.to_string().contains("does not verify")
                || err.to_string().contains("not issued by"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn empty_and_malformed_signatures_are_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let cert = release.cert();
        let data = sample_genesis_gz();

        for sig in [
            b"".to_vec(),
            b"not a signature".to_vec(),
            b"-----BEGIN PGP SIGNATURE-----\n\nzzzz\n-----END PGP SIGNATURE-----\n".to_vec(),
        ] {
            let _ = verify_detached_signature(&data, &sig, &cert)
                .expect_err("malformed signature must be rejected");
        }
    }

    #[test]
    fn truncated_signature_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let mut sig = release.sign(&data);
        sig.truncate(sig.len() / 2);

        let _ = verify_detached_signature(&data, &sig, &release.cert())
            .expect_err("truncated signature must be rejected");
    }

    #[test]
    fn public_key_block_in_place_of_a_signature_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();

        let _ = verify_detached_signature(&data, release.armored.as_bytes(), &release.cert())
            .expect_err("a certificate is not a detached signature");
    }

    #[test]
    fn weak_digest_signature_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();

        // rPGP refuses to *produce* a SHA-1 EdDSA signature, so downgrade a genuine one the way
        // an attacker betting on a weak digest would.
        let genuine = parse_sig(&release.sign(&data));
        let mut config = genuine.signature.config().expect("config").clone();
        config.hash_alg = HashAlgorithm::Sha1;
        let weak = Signature::from_config(
            config,
            genuine.signature.signed_hash_value().expect("hash value"),
            genuine.signature.signature().expect("sig bytes").clone(),
        )
        .expect("build downgraded signature");

        let err =
            verify_detached_signature(&data, &armor(DetachedSignature::new(weak)), &release.cert())
                .expect_err("SHA-1 signature must be rejected");
        assert!(
            err.to_string().contains("rejected digest algorithm"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn text_mode_signature_is_rejected() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let sig = release.sign_text(&data);

        let err = verify_detached_signature(&data, &sig, &release.cert())
            .expect_err("text mode signature must be rejected");
        assert!(
            err.to_string().contains("unexpected signature type"),
            "unexpected error: {err}"
        );
    }

    // ---------------------------------------------------------------------
    // Digest pinning
    // ---------------------------------------------------------------------

    #[test]
    fn digest_pin_mismatch_is_rejected_even_with_a_valid_signature() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let sig = release.sign(&data);

        let pinned = GenesisArtifact {
            sha256: Some([0x11; 32]),
            ..artifact()
        };
        let err =
            authenticate(&data, &sig, &pinned, &release.cert()).expect_err("digest pin must apply");
        assert!(
            err.to_string().contains("digest mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn matching_digest_pin_is_accepted() {
        let release = TestKey::new("Release <release@example.com>");
        let data = sample_genesis_gz();
        let sig = release.sign(&data);

        let pinned = GenesisArtifact {
            sha256: Some(Sha256::digest(&data).into()),
            ..artifact()
        };
        authenticate(&data, &sig, &pinned, &release.cert()).expect("pinned digest must verify");
    }

    // ---------------------------------------------------------------------
    // Cache handling
    // ---------------------------------------------------------------------

    #[test]
    fn download_verifies_then_caches_and_reuses_the_cache() {
        let release = TestKey::new("Release <release@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let gz = sample_genesis_gz();
        let asc = release.sign(&gz);
        let artifact = GenesisArtifact {
            sha256: Some(Sha256::digest(&gz).into()),
            ..artifact()
        };
        let feed = FakeRelease::new(artifact, gz.clone(), asc.clone());

        let genesis = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|url, max| feed.fetch(url, max),
        )
        .expect("first load must succeed");
        assert_eq!(genesis.config.chain_id, 1337);
        assert_eq!(feed.calls(), 2);
        assert_eq!(fs::read(dir.path().join(artifact.gz_name())).unwrap(), gz);
        assert_eq!(fs::read(dir.path().join(artifact.asc_name())).unwrap(), asc);

        // Second load is served from cache: the fetcher is not touched again.
        load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|url, max| feed.fetch(url, max),
        )
        .expect("cached load must succeed");
        assert_eq!(feed.calls(), 2);
    }

    #[test]
    fn replaced_cache_is_discarded_and_refetched() {
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let gz = sample_genesis_gz();
        let artifact = artifact();
        let feed = FakeRelease::new(artifact, gz.clone(), release.sign(&gz));

        // An attacker-controlled genesis, signed with an attacker key, planted in the cache.
        let evil_gz = substituted_genesis_gz();
        plant_cache(dir.path(), &artifact, &evil_gz, &attacker.sign(&evil_gz));

        let genesis = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|url, max| feed.fetch(url, max),
        )
        .expect("must fall back to the release");

        // The substituted chain id never reaches the caller, and the cache is repaired.
        assert_eq!(genesis.config.chain_id, 1337);
        assert_eq!(feed.calls(), 2);
        assert_eq!(fs::read(dir.path().join(artifact.gz_name())).unwrap(), gz);
    }

    #[test]
    fn cached_artifact_tampered_in_place_is_rejected() {
        // The signature is genuine but the compressed bytes next to it were edited.
        let release = TestKey::new("Release <release@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();
        let gz = sample_genesis_gz();
        let asc = release.sign(&gz);

        let mut tampered = substituted_genesis_gz();
        tampered.push(0);
        plant_cache(dir.path(), &artifact, &tampered, &asc);

        let _ = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|_, _| bail!("network unavailable"),
        )
        .expect_err("a signature from another artifact must not authenticate these bytes");
    }

    #[test]
    fn replaced_cache_is_not_accepted_when_the_release_is_unreachable() {
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();

        let evil_gz = substituted_genesis_gz();
        plant_cache(dir.path(), &artifact, &evil_gz, &attacker.sign(&evil_gz));

        let _ = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|_, _| bail!("network unavailable"),
        )
        .expect_err("an unauthenticated cache must never be used");
    }

    #[test]
    fn cache_with_a_missing_signature_is_not_accepted() {
        let release = TestKey::new("Release <release@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();
        let gz = sample_genesis_gz();
        fs::write(dir.path().join(artifact.gz_name()), &gz).unwrap();

        // Cached data with no signature next to it must not be trusted, even though the bytes
        // themselves are genuine.
        let _ = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|_, _| bail!("network unavailable"),
        )
        .expect_err("missing signature must fail closed");

        // With the release reachable the same bytes are accepted only after re-fetching.
        let feed = FakeRelease::new(artifact, gz.clone(), release.sign(&gz));
        load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|url, max| feed.fetch(url, max),
        )
        .expect("re-fetch must succeed");
        assert_eq!(feed.calls(), 2);
    }

    #[test]
    fn downloaded_artifact_that_fails_verification_is_not_cached() {
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();
        let gz = sample_genesis_gz();
        let feed = FakeRelease::new(artifact, gz.clone(), attacker.sign(&gz));

        let _ = load_genesis_verified(
            dir.path(),
            &artifact,
            &release.armored,
            &release.fingerprint,
            &|url, max| feed.fetch(url, max),
        )
        .expect_err("a badly signed download must fail closed");

        assert!(!dir.path().join(artifact.gz_name()).exists());
        assert!(!dir.path().join(artifact.asc_name()).exists());
    }

    #[test]
    fn oversized_cached_artifact_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = artifact();
        plant_cache(
            dir.path(),
            &artifact,
            &sample_genesis_gz(),
            &vec![0u8; MAX_SIGNATURE_BYTES + 1],
        );

        let err = read_capped(&dir.path().join(artifact.asc_name()), MAX_SIGNATURE_BYTES)
            .expect_err("oversized signature must be rejected");
        assert!(err.to_string().contains("limit"), "unexpected error: {err}");
        assert!(read_cached_pair(
            &dir.path().join(artifact.gz_name()),
            &dir.path().join(artifact.asc_name())
        )
        .is_none());
    }

    // ---------------------------------------------------------------------
    // Built-in networks
    // ---------------------------------------------------------------------

    #[test]
    fn built_in_networks_have_pinned_digests_and_expected_asset_names() {
        for (name, artifact) in BUILT_IN_GENESIS_ARTIFACTS {
            assert!(
                artifact.sha256.is_some(),
                "{name} genesis artifact has no digest pin"
            );
            assert!(
                artifact.gz_name().starts_with("genesis-")
                    && artifact.gz_name().ends_with(".json.gz"),
                "{name}: unexpected asset name {}",
                artifact.gz_name()
            );
            assert_eq!(
                artifact.asc_name(),
                format!("{}.asc", artifact.gz_name()),
                "{name}: signature name must follow the artifact name"
            );
            for url in [artifact.gz_url(), artifact.asc_url()] {
                assert!(
                    url.starts_with(
                        "https://github.com/fluentlabs-xyz/fluentbase/releases/download/"
                    ),
                    "{name}: unexpected download URL {url}"
                );
            }
        }
    }

    /// Every built-in network must refuse a cache that was swapped for a differently signed
    /// artifact, whether or not the release is reachable.
    #[test]
    fn every_built_in_network_rejects_a_substituted_cache() {
        let release = TestKey::new("Release <release@example.com>");
        let attacker = TestKey::new("Attacker <attacker@example.com>");
        let evil_gz = substituted_genesis_gz();
        let evil_asc = attacker.sign(&evil_gz);

        for (name, artifact) in BUILT_IN_GENESIS_ARTIFACTS {
            let dir = tempfile::tempdir().unwrap();
            plant_cache(dir.path(), artifact, &evil_gz, &evil_asc);

            let offline = load_genesis_verified(
                dir.path(),
                artifact,
                &release.armored,
                &release.fingerprint,
                &|_, _| bail!("network unavailable"),
            );
            assert!(
                offline.is_err(),
                "{name}: substituted cache must not be accepted"
            );

            // Even with a reachable release serving the very same substituted bytes, the digest
            // pin keeps them out.
            let feed = FakeRelease::new(**artifact, evil_gz.clone(), evil_asc.clone());
            let online = load_genesis_verified(
                dir.path(),
                artifact,
                &release.armored,
                &release.fingerprint,
                &|url, max| feed.fetch(url, max),
            );
            assert!(
                online.is_err(),
                "{name}: substituted release content must not be accepted"
            );
            assert!(
                !dir.path().join(artifact.gz_name()).exists(),
                "{name}: rejected content must not be left in the cache"
            );
        }
    }

    /// The detached signatures actually published alongside each built-in network's genesis.
    ///
    /// Refresh these together with the tag and digest pin in `chainspec.rs`.
    const PUBLISHED_SIGNATURES: &[(&str, &[u8])] = &[
        (
            "fluent-devnet",
            include_bytes!("../testdata/genesis-v0.5.7.json.gz.asc"),
        ),
        (
            "fluent-testnet",
            include_bytes!("../testdata/genesis-v0.3.4-dev.json.gz.asc"),
        ),
        (
            "fluent-mainnet",
            include_bytes!("../testdata/genesis-mainnet-v1.0.0.json.gz.asc"),
        ),
    ];

    /// Ties the pinned key to production: every published signature must be a binary,
    /// strong-digest signature issued by the key embedded in this file.
    #[test]
    fn published_signatures_are_issued_by_the_pinned_release_key() {
        let cert = load_release_cert(FLUENT_RELEASE_PUBKEY_ASC, FLUENT_RELEASE_KEY_FINGERPRINT)
            .expect("embedded key must load");
        let candidates = signing_candidates(&cert);

        for (name, artifact) in BUILT_IN_GENESIS_ARTIFACTS {
            let (_, asc) = PUBLISHED_SIGNATURES
                .iter()
                .find(|(network, _)| network == name)
                .unwrap_or_else(|| panic!("{name}: no published signature checked in"));

            let detached = parse_sig(asc);
            let sig = &detached.signature;
            assert_eq!(
                sig.typ(),
                Some(SignatureType::Binary),
                "{name}: {} is not a binary signature",
                artifact.asc_name()
            );
            assert!(
                ACCEPTED_HASH_ALGORITHMS.contains(&sig.hash_alg().expect("digest algorithm")),
                "{name}: {} uses an unaccepted digest",
                artifact.asc_name()
            );
            assert!(
                candidates.iter().any(|candidate| {
                    sig.issuer_fingerprint()
                        .iter()
                        .any(|fpr| **fpr == candidate.fingerprint())
                        || sig
                            .issuer_key_id()
                            .iter()
                            .any(|id| **id == candidate.legacy_key_id())
                }),
                "{name}: {} was not issued by the pinned release key",
                artifact.asc_name()
            );
        }
    }

    /// A validly signed artifact whose bytes are not the pinned ones is still refused.
    #[test]
    fn every_built_in_network_rejects_content_that_misses_its_digest_pin() {
        let release = TestKey::new("Release <release@example.com>");
        let cert = release.cert();
        let gz = sample_genesis_gz();
        let asc = release.sign(&gz);

        for (name, artifact) in BUILT_IN_GENESIS_ARTIFACTS {
            let err = authenticate(&gz, &asc, artifact, &cert)
                .expect_err("digest pin must reject foreign content")
                .to_string();
            assert!(
                err.contains("digest mismatch"),
                "{name}: unexpected error: {err}"
            );
        }
    }
}

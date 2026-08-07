//! Fixtures for exercising the fail-closed paths from other crates' tests.
//!
//! Enabled by this crate's own tests and by the `test-support` feature, so a consumer can build a
//! throwaway release — a key, an artifact, and a signature over it — without re-implementing any of
//! it. Never enable `test-support` outside `[dev-dependencies]`.

use crate::{
    error::FetchError,
    key::{fingerprint_hex, ReleaseKey},
    ReleaseAsset,
};
use flate2::{write::GzEncoder, Compression};
use pgp::{
    composed::{DetachedSignature, KeyType, SecretKeyParamsBuilder, SignedSecretKey},
    crypto::hash::HashAlgorithm,
    types::{KeyDetails as _, Password},
};
use std::{cell::RefCell, fs, io::Write as _, path::Path};

/// Gzips `payload` the way the release workflow packs an artifact.
pub fn gzip(payload: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(payload).unwrap();
    encoder.finish().unwrap()
}

/// A throwaway release identity: secret key, armored certificate, and its fingerprint.
pub struct TestKey {
    secret: SignedSecretKey,
    /// The armored public certificate, as it would be embedded in a verifier.
    pub armored: String,
    /// Uppercase hex fingerprint of the certificate.
    pub fingerprint: String,
}

impl TestKey {
    pub fn new(user_id: &str) -> Self {
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

    /// The identity a test treats as the real release signer.
    pub fn release() -> Self {
        Self::new("Release <release@example.com>")
    }

    /// An identity a test treats as hostile.
    pub fn attacker() -> Self {
        Self::new("Attacker <attacker@example.com>")
    }

    /// A [`ReleaseKey`] pinned to this identity.
    pub fn key(&self) -> ReleaseKey {
        ReleaseKey::new(&self.armored, &self.fingerprint).expect("test key must load")
    }

    /// A detached binary SHA-256 signature over `data`.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        self.sign_with(SignatureMode::Binary, data)
    }

    /// A detached text-mode signature over `data`, which verification must refuse.
    pub fn sign_text(&self, data: &[u8]) -> Vec<u8> {
        self.sign_with(SignatureMode::Text, data)
    }

    fn sign_with(&self, mode: SignatureMode, data: &[u8]) -> Vec<u8> {
        let sign = match mode {
            SignatureMode::Binary => DetachedSignature::sign_binary_data,
            SignatureMode::Text => DetachedSignature::sign_text_data,
        };
        sign(
            rand_08::rngs::OsRng,
            &self.secret.primary_key,
            &Password::empty(),
            HashAlgorithm::Sha256,
            data,
        )
        .expect("sign")
        .to_armored_bytes(Default::default())
        .expect("armor signature")
    }
}

enum SignatureMode {
    Binary,
    Text,
}

/// A fetcher standing in for one release's asset pair, counting how often it was called.
pub struct FakeRelease {
    assets: RefCell<Vec<(String, Vec<u8>)>>,
    calls: RefCell<usize>,
}

impl Default for FakeRelease {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeRelease {
    pub fn new() -> Self {
        Self {
            assets: RefCell::new(Vec::new()),
            calls: RefCell::new(0),
        }
    }

    /// Publishes `bytes` at `asset`'s URL and `asc` at its signature URL.
    pub fn publish(self, asset: &ReleaseAsset, bytes: Vec<u8>, asc: Vec<u8>) -> Self {
        self.assets.borrow_mut().push((asset.url(), bytes));
        self.assets.borrow_mut().push((asset.signature_url(), asc));
        self
    }

    pub fn fetch(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        *self.calls.borrow_mut() += 1;
        let body = self
            .assets
            .borrow()
            .iter()
            .find(|(published, _)| published == url)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| FetchError::not_found(format!("404 for {url}")))?;
        if body.len() > max_bytes {
            return Err(FetchError::new(format!("{url} is over the byte limit")));
        }
        Ok(body)
    }

    pub fn calls(&self) -> usize {
        *self.calls.borrow()
    }
}

/// A fetcher that always fails, standing in for an unreachable release.
pub fn offline(_url: &str, _max_bytes: usize) -> Result<Vec<u8>, FetchError> {
    Err(FetchError::new("network unavailable"))
}

/// Plants an asset pair in `dir` as if an earlier run had cached it.
pub fn plant_cache(dir: &Path, asset: &ReleaseAsset, bytes: &[u8], asc: &[u8]) {
    fs::write(dir.join(asset.name()), bytes).unwrap();
    fs::write(dir.join(asset.signature_name()), asc).unwrap();
}

//! Fail-closed loading of the release artifact the upgrade payloads are cut from.
//!
//! Everything this tool signs with the operator wallet is derived from a genesis `.json.gz`
//! published on GitHub releases, so that artifact is the real input to a privileged action. It is
//! treated as untrusted until it has been authenticated against the release key pinned in
//! [`fluentbase_release_verify`]:
//!
//! * the detached OpenPGP signature is required for both cached and downloaded artifacts, and is
//!   checked over the exact bytes that are later parsed;
//! * when the release publishes a signed digest manifest, it is verified too and must list this
//!   exact asset, at this exact digest, for this exact release — that is what binds the artifact to
//!   a network and rules out replaying another release's manifest;
//! * releases predating the manifest (pre-`v1.3.x`) are accepted on the detached signature alone,
//!   and the run says so loudly and records it in the result manifest;
//! * any failure aborts before the wallet is loaded or a transaction is built.

use alloy_genesis::Genesis;
use anyhow::{anyhow, Context, Result};
use fluentbase_release_verify::{
    bounded_http_fetch, load_verified, parse_genesis_gz, FetchError, Fetcher, ReleaseAsset,
    ReleaseKey, ReleaseManifest, VerifyError,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Timeout for a single artifact download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// How the release's signed manifest was (or was not) able to vouch for the artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManifestBinding {
    /// The manifest verified and lists this asset at this digest for this release.
    Verified,
    /// This release publishes no manifest; only the detached signature bound the artifact.
    Unavailable { reason: String },
}

/// What the artifact's provenance was proven to be. Recorded in the run's result manifest so an
/// upgrade can be audited after the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseProvenance {
    /// Release tag the artifact came from.
    pub tag: String,
    /// Published asset name.
    pub asset: String,
    /// SHA-256 the artifact authenticated at.
    pub sha256: String,
    /// Source commit, when the manifest supplied one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub manifest: ManifestBinding,
}

/// An authenticated release artifact, parsed.
#[derive(Debug)]
pub struct VerifiedRelease {
    pub genesis: Genesis,
    pub provenance: ReleaseProvenance,
}

/// Loads and authenticates the genesis artifact of release `tag`.
///
/// Runs the blocking verification off the async runtime; the CLI has no other work to overlap with
/// it, and a blocking HTTP client keeps the trust-critical path free of task scheduling.
pub async fn load_release(
    tag: &str,
    channel: Option<&str>,
    cache_dir: &Path,
) -> Result<VerifiedRelease> {
    let tag = tag.to_owned();
    let channel = channel.map(str::to_owned);
    let cache_dir = cache_dir.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let key = ReleaseKey::fluent().context("release key is unusable")?;
        load_release_blocking(&tag, channel.as_deref(), &cache_dir, &key, &blocking_fetch)
    })
    .await
    .context("release verification task panicked")?
}

/// The verification flow, with the HTTP client and trust root injected so tests can drive it.
pub(crate) fn load_release_blocking(
    tag: &str,
    channel: Option<&str>,
    cache_dir: &Path,
    key: &ReleaseKey,
    fetch: &Fetcher<'_>,
) -> Result<VerifiedRelease> {
    let asset = ReleaseAsset::genesis(tag.to_owned(), channel)
        .context("invalid genesis release tag or channel")?;

    let artifact = load_verified(Some(cache_dir), &asset, key, fetch)
        .with_context(|| format!("refusing to use {}: it is not authentic", asset.name()))?;

    let (manifest, binding) = load_manifest(tag, cache_dir, key, fetch)?;
    if let Some(manifest) = &manifest {
        manifest
            .check(tag, asset.name(), &artifact.sha256)
            .with_context(|| {
                format!(
                    "refusing to use {}: the release manifest does not vouch for it",
                    asset.name()
                )
            })?;
    }

    let genesis = parse_genesis_gz(&artifact).context("parsing authenticated genesis")?;

    Ok(VerifiedRelease {
        genesis,
        provenance: ReleaseProvenance {
            tag: tag.to_owned(),
            asset: asset.name().to_owned(),
            sha256: artifact.sha256_hex(),
            commit: manifest.as_ref().map(|m| m.commit().to_owned()),
            manifest: binding,
        },
    })
}

/// Loads the release's signed digest manifest, if it publishes one.
///
/// A manifest that returns `404 Not Found` is treated as "this release has none" — the detached
/// signature has already bound the artifact, and older releases genuinely predate manifests. Any
/// other transport, authentication, or parse failure is fatal rather than a verification downgrade.
fn load_manifest(
    tag: &str,
    cache_dir: &Path,
    key: &ReleaseKey,
    fetch: &Fetcher<'_>,
) -> Result<(Option<ReleaseManifest>, ManifestBinding)> {
    let asset = ReleaseAsset::manifest(tag.to_owned()).context("invalid release tag")?;

    let artifact = match load_verified(Some(cache_dir), &asset, key, fetch) {
        Ok(artifact) => artifact,
        Err(VerifyError::Fetch { url, source }) if source.is_not_found() && url == asset.url() => {
            return Ok((
                None,
                ManifestBinding::Unavailable {
                    reason: source.to_string(),
                },
            ))
        }
        Err(err) => {
            return Err(anyhow!(err)).with_context(|| {
                format!(
                    "refusing to continue: {} did not authenticate",
                    asset.name()
                )
            })
        }
    };

    let manifest = ReleaseManifest::parse(&artifact.bytes)
        .map_err(|err| anyhow!(err))
        .with_context(|| format!("parsing {}", asset.name()))?;

    Ok((Some(manifest), ManifestBinding::Verified))
}

/// Downloads `url` into memory, refusing responses larger than `max_bytes`.
fn blocking_fetch(url: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
    bounded_http_fetch(
        "fluent-runtime-upgrade/1.0",
        DOWNLOAD_TIMEOUT,
        url,
        max_bytes,
    )
}

/// Where authenticated artifacts are cached: alongside the working directory, as before.
pub fn cache_dir() -> PathBuf {
    PathBuf::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_release_verify::test_support::{
        gzip, offline, plant_cache, FakeRelease, TestKey,
    };
    use sha2::Digest as _;

    const TAG: &str = "v1.3.2";

    fn genesis_gz() -> Vec<u8> {
        gzip(
            br#"{"config":{"chainId":20993},"alloc":{},"gasLimit":"0x1c9c380","difficulty":"0x0"}"#,
        )
    }

    fn substituted_gz() -> Vec<u8> {
        gzip(br#"{"config":{"chainId":31337},"alloc":{},"gasLimit":"0x1","difficulty":"0x0"}"#)
    }

    fn manifest_for(tag: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = format!("version={tag}\ncommit=deadbeef\n\n[compressed]\n");
        for (name, bytes) in entries {
            out.push_str(&format!(
                "{}  ./artifacts/{name}\n",
                hex::encode(<[u8; 32]>::from(sha2::Sha256::digest(bytes)))
            ));
        }
        out.into_bytes()
    }

    /// A release that publishes a genesis artifact and a matching signed manifest.
    fn full_release(
        release: &TestKey,
        tag: &str,
        channel: Option<&str>,
        gz: &[u8],
    ) -> (ReleaseAsset, FakeRelease) {
        let asset =
            ReleaseAsset::genesis(tag.to_owned(), channel).expect("valid test genesis asset");
        let manifest_asset =
            ReleaseAsset::manifest(tag.to_owned()).expect("valid test manifest asset");
        let manifest = manifest_for(tag, &[(asset.name(), gz)]);
        let feed = FakeRelease::new()
            .publish(&asset, gz.to_vec(), release.sign(gz))
            .publish(&manifest_asset, manifest.clone(), release.sign(&manifest));
        (asset, feed)
    }

    #[test]
    fn valid_release_is_accepted_and_manifest_bound() {
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let (asset, feed) = full_release(&release, TAG, None, &gz);

        let loaded = load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect("a valid release must be accepted");

        assert_eq!(loaded.genesis.config.chain_id, 20993);
        assert_eq!(loaded.provenance.manifest, ManifestBinding::Verified);
        assert_eq!(loaded.provenance.asset, asset.name());
        assert_eq!(loaded.provenance.commit.as_deref(), Some("deadbeef"));
        assert_eq!(
            loaded.provenance.sha256,
            hex::encode(<[u8; 32]>::from(sha2::Sha256::digest(&gz)))
        );
    }

    #[test]
    fn same_name_cache_substitution_is_rejected() {
        // The reported hole: drop a same-name file next to the tool and let the next run use it.
        let release = TestKey::release();
        let attacker = TestKey::attacker();
        let dir = tempfile::tempdir().unwrap();
        let evil = substituted_gz();
        let asset = ReleaseAsset::genesis(TAG.to_owned(), None).unwrap();
        plant_cache(dir.path(), &asset, &evil, &attacker.sign(&evil));

        load_release_blocking(TAG, None, dir.path(), &release.key(), &offline)
            .expect_err("a substituted cache must never be used");
        assert!(
            !dir.path().join(asset.name()).exists(),
            "the rejected cache entry must be gone"
        );
    }

    #[test]
    fn substituted_cache_is_replaced_by_the_authentic_release() {
        let release = TestKey::release();
        let attacker = TestKey::attacker();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let (asset, feed) = full_release(&release, TAG, None, &gz);

        let evil = substituted_gz();
        plant_cache(dir.path(), &asset, &evil, &attacker.sign(&evil));

        let loaded = load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect("must fall back to the release");
        assert_eq!(loaded.genesis.config.chain_id, 20993);
        assert_eq!(std::fs::read(dir.path().join(asset.name())).unwrap(), gz);
    }

    #[test]
    fn modified_gzip_is_rejected() {
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let (asset, _) = full_release(&release, TAG, None, &gz);

        // A single flipped byte in an otherwise genuine artifact.
        let mut modified = gz.clone();
        *modified.last_mut().unwrap() ^= 0x01;
        let manifest_asset = ReleaseAsset::manifest(TAG.to_owned()).unwrap();
        let manifest = manifest_for(TAG, &[(asset.name(), &gz)]);
        let feed = FakeRelease::new()
            .publish(&asset, modified, release.sign(&gz))
            .publish(&manifest_asset, manifest.clone(), release.sign(&manifest));

        load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect_err("a modified gzip must be rejected");
    }

    #[test]
    fn signature_failure_is_rejected() {
        let release = TestKey::release();
        let attacker = TestKey::attacker();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let (asset, _) = full_release(&release, TAG, None, &gz);
        let feed = FakeRelease::new().publish(&asset, gz.clone(), attacker.sign(&gz));

        load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect_err("an artifact signed by another key must be rejected");
    }

    #[test]
    fn manifest_from_another_release_is_rejected() {
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis(TAG.to_owned(), None).unwrap();
        let manifest_asset = ReleaseAsset::manifest(TAG.to_owned()).unwrap();

        // Validly signed, but cut from a different release.
        let manifest = manifest_for("v1.3.1", &[(asset.name(), &gz)]);
        let feed = FakeRelease::new()
            .publish(&asset, gz.clone(), release.sign(&gz))
            .publish(&manifest_asset, manifest.clone(), release.sign(&manifest));

        let err = load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect_err("a manifest from another release must be rejected");
        assert!(
            format!("{err:#}").contains("does not vouch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn manifest_for_another_network_is_rejected() {
        // The mainnet and devnet artifacts of one release differ only by asset name, so asking for
        // the mainnet asset against a manifest that only lists the devnet one must fail.
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let devnet = ReleaseAsset::genesis(TAG.to_owned(), None).unwrap();
        let mainnet = ReleaseAsset::genesis(TAG.to_owned(), Some("mainnet")).unwrap();
        let manifest_asset = ReleaseAsset::manifest(TAG.to_owned()).unwrap();

        let manifest = manifest_for(TAG, &[(devnet.name(), &gz)]);
        let feed = FakeRelease::new()
            .publish(&mainnet, gz.clone(), release.sign(&gz))
            .publish(&manifest_asset, manifest.clone(), release.sign(&manifest));

        let err = load_release_blocking(
            TAG,
            Some("mainnet"),
            dir.path(),
            &release.key(),
            &|url, max| feed.fetch(url, max),
        )
        .expect_err("a manifest that omits this network's asset must be rejected");
        assert!(
            format!("{err:#}").contains("does not vouch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn tampered_manifest_is_fatal_not_ignored() {
        let release = TestKey::release();
        let attacker = TestKey::attacker();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis(TAG.to_owned(), None).unwrap();
        let manifest_asset = ReleaseAsset::manifest(TAG.to_owned()).unwrap();
        let manifest = manifest_for(TAG, &[(asset.name(), &gz)]);

        let feed = FakeRelease::new()
            .publish(&asset, gz.clone(), release.sign(&gz))
            .publish(&manifest_asset, manifest.clone(), attacker.sign(&manifest));

        let err = load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect_err("a manifest that fails to authenticate must abort the run");
        assert!(
            format!("{err:#}").contains("did not authenticate"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn unsigned_manifest_is_fatal_not_treated_as_absent() {
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis(TAG.to_owned(), None).unwrap();
        let manifest_asset = ReleaseAsset::manifest(TAG.to_owned()).unwrap();
        let manifest = manifest_for(TAG, &[(asset.name(), &gz)]);
        let feed = FakeRelease::new()
            .publish(&asset, gz.clone(), release.sign(&gz))
            // Publish the manifest body without its detached signature.
            .publish(&manifest_asset, manifest.clone(), Vec::new());

        let err = load_release_blocking(TAG, None, dir.path(), &release.key(), &|url, max| {
            if url == manifest_asset.signature_url() {
                Err(FetchError::not_found("manifest signature returned 404"))
            } else {
                feed.fetch(url, max)
            }
        })
        .expect_err("a manifest without its detached signature must abort the run");
        assert!(
            format!("{err:#}").contains("did not authenticate"),
            "{err:#}"
        );
    }

    #[test]
    fn release_without_a_manifest_is_accepted_and_flagged() {
        // Pre-v1.3.x releases publish no manifest; the detached signature still has to hold.
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis("v0.5.7".to_owned(), None).unwrap();
        let feed = FakeRelease::new().publish(&asset, gz.clone(), release.sign(&gz));

        let loaded =
            load_release_blocking("v0.5.7", None, dir.path(), &release.key(), &|url, max| {
                feed.fetch(url, max)
            })
            .expect("a signed artifact from a pre-manifest release must still load");
        assert!(matches!(
            loaded.provenance.manifest,
            ManifestBinding::Unavailable { .. }
        ));
        assert_eq!(loaded.provenance.commit, None);
    }

    #[test]
    fn manifest_transport_failure_is_fatal() {
        let release = TestKey::release();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis("v0.5.7".to_owned(), None).unwrap();
        let feed = FakeRelease::new().publish(&asset, gz.clone(), release.sign(&gz));

        let err = load_release_blocking("v0.5.7", None, dir.path(), &release.key(), &|url, max| {
            if url.contains("genesis-manifest-") {
                Err(FetchError::new("network timeout"))
            } else {
                feed.fetch(url, max)
            }
        })
        .expect_err("a timeout must not masquerade as a release without a manifest");

        assert!(format!("{err:#}").contains("network timeout"), "{err:#}");
    }

    #[test]
    fn unmanifested_release_still_requires_a_valid_signature() {
        let release = TestKey::release();
        let attacker = TestKey::attacker();
        let dir = tempfile::tempdir().unwrap();
        let gz = genesis_gz();
        let asset = ReleaseAsset::genesis("v0.5.7".to_owned(), None).unwrap();
        let feed = FakeRelease::new().publish(&asset, gz.clone(), attacker.sign(&gz));

        load_release_blocking("v0.5.7", None, dir.path(), &release.key(), &|url, max| {
            feed.fetch(url, max)
        })
        .expect_err("a missing manifest must not weaken the signature requirement");
    }
}

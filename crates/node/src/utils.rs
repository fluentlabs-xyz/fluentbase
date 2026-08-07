//! Genesis retrieval for the built-in networks.
//!
//! Built-in networks (devnet / testnet / mainnet) take their genesis state from a `.json.gz` asset
//! published on GitHub releases. Everything about trusting that asset — the pinned release key,
//! detached signature verification, digest pinning, and the fail-closed cache handling — lives in
//! [`fluentbase_release_verify`]. This module only supplies the HTTP client and the cache location.

use alloy_genesis::Genesis;
use eyre::WrapErr as _;
use fluentbase_release_verify::{
    bounded_http_fetch, load_verified, parse_genesis_gz, FetchError, ReleaseAsset, ReleaseKey,
};
use std::{path::PathBuf, time::Duration};

/// Timeout for a single artifact download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Loads the genesis of a built-in network, authenticating the artifact before use.
///
/// This is intentionally synchronous because it runs during CLI startup / chainspec selection.
pub fn download_and_cache_genesis_verified(asset: &ReleaseAsset) -> eyre::Result<Genesis> {
    let cache_dir = genesis_cache_dir()?;
    let key = ReleaseKey::fluent().wrap_err("release key is unusable")?;

    println!("Checking genesis for release {}...", asset.tag());
    let artifact =
        load_verified(Some(&cache_dir), asset, &key, &http_fetch).wrap_err_with(|| {
            format!(
                "genesis authentication failed for {} — refusing to start",
                asset.name()
            )
        })?;
    println!(
        "Using genesis {} (sha256 {})",
        artifact.name,
        artifact.sha256_hex()
    );

    parse_genesis_gz(&artifact).map_err(Into::into)
}

/// Where to cache genesis files.
fn genesis_cache_dir() -> eyre::Result<PathBuf> {
    let proj = directories::ProjectDirs::from("xyz", "fluentlabs", "fluent")
        .ok_or_else(|| eyre::eyre!("cannot determine cache directory"))?;
    Ok(proj.cache_dir().join("genesis"))
}

/// Downloads `url` into memory, refusing responses larger than `max_bytes`.
fn http_fetch(url: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
    // NOTE: blocking client avoids pulling tokio into a CLI dependency tree.
    bounded_http_fetch("fluent-chainspec/1.0", DOWNLOAD_TIMEOUT, url, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chainspec::built_in_genesis_assets;

    #[test]
    fn built_in_networks_have_pinned_digests_and_expected_asset_names() {
        for (name, asset) in built_in_genesis_assets() {
            assert!(
                asset.sha256().is_some(),
                "{name} genesis asset has no digest pin"
            );
            assert!(
                asset.name().starts_with("genesis-") && asset.name().ends_with(".json.gz"),
                "{name}: unexpected asset name {}",
                asset.name()
            );
            assert_eq!(
                asset.signature_name(),
                format!("{}.asc", asset.name()),
                "{name}: signature name must follow the asset name"
            );
            for url in [asset.url(), asset.signature_url()] {
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
    /// artifact, whether or not the release is reachable. The verification itself is covered by
    /// `fluentbase-release-verify`; this pins the guarantee to the concrete network table.
    #[test]
    fn every_built_in_network_rejects_a_substituted_cache() {
        use fluentbase_release_verify::authenticate;

        let key = ReleaseKey::fluent().expect("embedded key must load");
        let evil = b"substituted genesis".to_vec();
        let no_signature = b"-----BEGIN PGP SIGNATURE-----\n\nzz\n-----END PGP SIGNATURE-----\n";

        for (name, asset) in built_in_genesis_assets() {
            let err = authenticate(&evil, no_signature, &asset, &key)
                .expect_err("substituted content must not authenticate");
            assert!(
                err.to_string().contains("digest mismatch"),
                "{name}: unexpected error: {err}"
            );
        }
    }
}

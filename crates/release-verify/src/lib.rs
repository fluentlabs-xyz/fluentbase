//! Fail-closed authentication of Fluent release artifacts.
//!
//! Release assets (`genesis-*.json.gz`, the digest manifest, …) are published on GitHub together
//! with detached OpenPGP signatures. Anything derived from them — genesis allocations, system
//! contract code, runtime-upgrade payloads — is only as trustworthy as that signature check, so
//! this crate treats every artifact as untrusted input until it has been authenticated against the
//! release key pinned in [`key`].
//!
//! The rules are deliberately fail-closed:
//!
//! * artifacts are held in memory until authentication succeeds — nothing is decompressed, parsed,
//!   or written to a cache before that;
//! * the signature is checked over the exact bytes that are later used, so there is no window
//!   between verification and use;
//! * cached files get the same treatment as freshly downloaded ones, and are discarded rather than
//!   trusted when they fail;
//! * digest pins and the signed [`manifest`] are checked in addition to the signature, never
//!   instead of it;
//! * every read, parse, signer, or signature error is an error. There is no bypass switch.
//!
//! HTTP lives with the caller: [`load_verified`] takes a [`Fetcher`] closure, so the same
//! verification runs behind a blocking client in the node and an async one in the CLIs.
//!
//! ```no_run
//! use fluentbase_release_verify::{load_verified, parse_genesis_gz, ReleaseAsset, ReleaseKey};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let key = ReleaseKey::fluent()?;
//! let asset = ReleaseAsset::genesis("v1.3.2", None);
//! let artifact = load_verified(Some(std::path::Path::new("/tmp/cache")), &asset, &key, &|_, _| {
//!     unimplemented!("plug in an HTTP client")
//! })?;
//! let genesis = parse_genesis_gz(&artifact)?;
//! # let _ = genesis;
//! # Ok(())
//! # }
//! ```

pub mod asset;
pub mod error;
pub mod key;
pub mod load;
pub mod manifest;
pub mod signature;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use asset::{
    ReleaseAsset, MAX_ARTIFACT_BYTES, MAX_GENESIS_JSON_BYTES, MAX_MANIFEST_BYTES,
    MAX_SIGNATURE_BYTES, RELEASE_BASE_URL,
};
pub use error::{FetchError, Result, VerifyError};
pub use key::{ReleaseKey, FLUENT_RELEASE_KEY_FINGERPRINT, FLUENT_RELEASE_PUBKEY_ASC};
pub use load::{
    authenticate, decompress_gz, load_verified, parse_genesis_gz, read_capped, write_atomic,
    Fetcher, VerifiedArtifact,
};
pub use manifest::ReleaseManifest;
pub use signature::verify_detached_signature;

#[cfg(test)]
mod tests;

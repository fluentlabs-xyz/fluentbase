//! At-rest backends for the VALIDATOR BLS secret (the 32-byte signing scalar).
//!
//! Two on-disk shapes are unified here behind [`SecretBackend`]:
//!
//! - [`SecretBackend::Eip2335`] — the production path: an EIP-2335 keystore v4
//!   file decrypted with an operator password (`keystore.rs`).
//! - [`SecretBackend::Plaintext`] — the dev/test fallback: a bare/`0x`-prefixed
//!   hex file (`--dpos.bls-key-path`), forbidden on deployed chain-ids.
//!
//! Both yield raw secret bytes as `Zeroizing<Vec<u8>>`; the length / field
//! validation stays at the typed [`crate::keys::ValidatorBlsKeypair::from_secret_bytes`]
//! boundary. The per-epoch DKG SHARE is a different secret shape (a
//! variable-length `(DkgOutcome, Share)` tuple) with its own AEAD-at-rest codec
//! in `consensus/beacon/share_state.rs` — it does NOT route through this module.
//!
//! [`write_mode_0600`] is the single shared 0600-write helper (the validator
//! plaintext writer and the share persistor previously each carried their own
//! copy).

use crate::error::Error;
use std::path::Path;
use zeroize::Zeroizing;

/// An at-rest backend for the validator BLS secret.
pub enum SecretBackend<'a> {
    /// Bare/`0x`-prefixed hex (`--dpos.bls-key-path`, dev/test only).
    Plaintext,
    /// EIP-2335 keystore v4 unlocked with `password` (`--dpos.bls-keystore-path`).
    Eip2335 { password: &'a [u8] },
}

impl SecretBackend<'_> {
    /// Read the raw secret bytes from `path`, decrypting / hex-decoding per the
    /// backend. The caller validates the byte shape (length / non-zero /
    /// in-field) via [`crate::keys::ValidatorBlsKeypair::from_secret_bytes`].
    pub fn open(&self, path: &Path) -> Result<Zeroizing<Vec<u8>>, Error> {
        match self {
            SecretBackend::Plaintext => {
                let raw = Zeroizing::new(std::fs::read_to_string(path)?);
                let bytes = commonware_utils::from_hex_formatted(raw.trim()).ok_or(Error::InvalidHex)?;
                Ok(Zeroizing::new(bytes))
            }
            SecretBackend::Eip2335 { password } => {
                let raw = Zeroizing::new(std::fs::read_to_string(path)?);
                let ks = crate::keystore::EthKeystoreV4::from_json(&raw)?;
                ks.decrypt(password)
            }
        }
    }
}

/// Write `data` to `path` with mode 0600 (truncate-create). The single shared
/// 0600-write helper for both the validator plaintext key file and the on-disk
/// DKG-share persistor.
#[cfg(unix)]
pub fn write_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)
}

#[cfg(not(unix))]
pub fn write_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

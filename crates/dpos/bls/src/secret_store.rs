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
        // bug 13 read-side: refuse a group/other-accessible secret file (a
        // backup/rsync that recreated it at 0644) instead of silently loading it.
        reject_insecure_mode(path)?;
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

/// Reject a secret file that is group/other-accessible (`mode & 0o077`), mirroring
/// the p2p key loader (`p2p/src/lib.rs`) — an operator who restored a key/share dir
/// at 0644 fails loud at load instead of silently running with a leaked secret.
/// Non-unix is a no-op (no POSIX mode). Shared by the validator secret loader and
/// the DKG-share loader (`consensus/beacon/share_state.rs`).
#[cfg(unix)]
pub fn reject_insecure_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "secret file {} has insecure permissions {:o}; chmod 600 required \
                 (group/other access denied)",
                path.display(),
                mode & 0o777,
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn reject_insecure_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Write `data` to `path` with mode 0600 (truncate-create). The single shared
/// 0600-write helper for both the validator plaintext key file and the on-disk
/// DKG-share persistor.
#[cfg(unix)]
pub fn write_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `OpenOptions::mode` only applies at CREATION (bug 13): a truncate-open of a
    // pre-existing 0644 file keeps its loose mode, so tighten it explicitly.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(data)
}

#[cfg(not(unix))]
pub fn write_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Append `data` to `path`, creating it mode 0600 if absent. The append sibling of
/// [`write_mode_0600`] for the per-epoch DKG ceremony journal, whose records are
/// written one at a time as the ceremony progresses (an O_APPEND open avoids
/// re-reading the whole file on every record).
#[cfg(unix)]
pub fn append_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    // `OpenOptions::mode` only applies at CREATION (bug 13): tighten a pre-existing
    // loose-mode journal file explicitly.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(data)?;
    // fsync the appended record before returning: write_all only reaches the
    // page cache, so a crash before the OS flushes would lose the tail. The
    // journal loader tolerates a torn tail (no corruption), but syncing each
    // record minimizes lost dealings -> fewer unnecessary sit-outs on restart
    // (matches tempo, which sync()s after every append).
    file.sync_all()
}

#[cfg(not(unix))]
pub fn append_mode_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

#[cfg(all(test, unix))]
mod tests {
    use super::{append_mode_0600, reject_insecure_mode, write_mode_0600};
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "fluent_secret_store_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }
    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
    fn make_0644(path: &Path) {
        std::fs::write(path, b"stale").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    // bug 13: `OpenOptions::mode` is ignored on a pre-existing file, so both write
    // helpers must explicitly re-tighten a restored-0644 secret to 0600.
    #[test]
    fn write_mode_0600_tightens_a_preexisting_0644_file() {
        let p = temp_path("write");
        make_0644(&p);
        write_mode_0600(&p, b"secret").unwrap();
        assert_eq!(mode_of(&p), 0o600);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn append_mode_0600_tightens_a_preexisting_0644_file() {
        let p = temp_path("append");
        make_0644(&p);
        append_mode_0600(&p, b"secret").unwrap();
        assert_eq!(mode_of(&p), 0o600);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn loader_rejects_group_or_other_accessible_secret() {
        let p = temp_path("reject");
        make_0644(&p);
        let err = reject_insecure_mode(&p).expect_err("0644 must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("chmod 600"), "actionable message");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        reject_insecure_mode(&p).expect("0600 must be accepted");
        std::fs::remove_file(&p).ok();
    }
}

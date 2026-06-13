use commonware_codec::Encode;
use commonware_cryptography::bls12381::primitives::{group::Private, ops};
use rand_core::CryptoRngCore;

use crate::{error::Error, BlsPubkey, Variant, PUBKEY_BYTES, SECRET_BYTES};

/// A validator's BLS keypair (MinSig: 32 B scalar private, 96 B G2 public).
///
/// Construct via [`generate`](Self::generate) or [`from_secret_bytes`](Self::from_secret_bytes).
/// The secret is held in [`Private`], which internally uses a `Secret<Scalar>`
/// wrapper that zeroizes on drop and forces explicit `expose()` to access the
/// underlying scalar.
#[derive(Clone)]
pub struct ValidatorBlsKeypair {
    secret: Private,
    public: BlsPubkey,
}

impl ValidatorBlsKeypair {
    /// Generate a fresh keypair from cryptographically secure randomness.
    ///
    /// Internally draws 64 bytes of IKM and runs IETF BLS KeyGen
    /// (`blst_keygen`), which loops until the scalar is non-zero.
    pub fn generate<R: CryptoRngCore>(rng: &mut R) -> Self {
        let (secret, public) = ops::keypair::<R, Variant>(rng);
        Self { secret, public }
    }

    /// Reconstruct a keypair from raw 32-byte secret bytes (big-endian scalar).
    ///
    /// Returns [`Error::InvalidSecret`] if the bytes do not decode to a valid
    /// non-zero scalar in the BLS12-381 scalar field.
    pub fn from_secret_bytes(bytes: &[u8; SECRET_BYTES]) -> Result<Self, Error> {
        use commonware_codec::DecodeExt;
        // `bytes` is caller-owned; `Private::decode` moves the scalar into the
        // zeroizing `Secret<Scalar>` wrapper, so no extra Zeroizing temp needed.
        let secret = Private::decode(bytes.as_slice()).map_err(|_| Error::InvalidSecret)?;
        let public = ops::compute_public::<Variant>(&secret);
        Ok(Self { secret, public })
    }

    /// Compressed BLS public key (G2, 96 B for MinSig).
    pub fn public_bytes(&self) -> [u8; PUBKEY_BYTES] {
        let bytes = self.public.encode();
        // `BlsPubkey::SIZE == PUBKEY_BYTES` for MinSig (G2 compressed).
        bytes
            .as_ref()
            .try_into()
            .expect("BLS pubkey is exactly 96 bytes for MinSig")
    }

    /// Borrow the underlying private scalar for use with `ops::*` functions
    /// from `commonware_cryptography`.
    ///
    /// Prefer the high-level [`crate::pop::sign_pop`] / signing helpers; this
    /// accessor exists so downstream code can pass the
    /// private key into [`crate::scheme::build_signer`] without duplicating it.
    pub(crate) fn secret(&self) -> &Private {
        &self.secret
    }

    /// Load from a hex-encoded plaintext key file (bare or `0x`-prefixed,
    /// surrounding whitespace trimmed). Plaintext fallback for operators not
    /// using an EIP-2335 keystore; prefer [`Self::read_from_keystore`] for
    /// encrypted storage.
    pub fn read_from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Error> {
        use zeroize::{Zeroize, Zeroizing};
        let raw = Zeroizing::new(std::fs::read_to_string(path.as_ref())?);
        let bytes = Zeroizing::new(
            commonware_utils::from_hex_formatted(raw.trim()).ok_or(Error::InvalidHex)?,
        );
        let mut arr: [u8; SECRET_BYTES] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidLength)?;
        let result = Self::from_secret_bytes(&arr);
        arr.zeroize();
        result
    }

    /// Load from an EIP-2335 keystore file (version 4).
    pub fn read_from_keystore<P: AsRef<std::path::Path>>(
        path: P,
        password: &[u8],
    ) -> Result<Self, Error> {
        use zeroize::Zeroizing;
        let raw = Zeroizing::new(std::fs::read_to_string(path.as_ref())?);
        let ks = crate::keystore::EthKeystoreV4::from_json(&raw)?;
        let secret = ks.decrypt(password)?;
        Self::from_secret_bytes(&secret)
    }

    /// Plaintext fallback writer (symmetric counterpart of
    /// [`Self::read_from_file`]): writes the 32-byte scalar as lowercase bare
    /// hex (no `0x` prefix, no trailing newline). Writes an UNENCRYPTED private
    /// key, so it is gated behind the off-by-default `plaintext-keys` feature
    /// and cannot link into a binary that does not opt in (only the local
    /// devnet bootstrap does). Prefer an encrypted EIP-2335 keystore otherwise.
    /// Secret bytes never cross the crate boundary — a leaky downstream cannot
    /// dump them via `{:?}` / `tracing`. On Unix, sets file mode 0600 at create.
    #[cfg(feature = "plaintext-keys")]
    pub fn write_to_plaintext_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), Error> {
        use commonware_codec::Encode;
        use zeroize::{Zeroize, Zeroizing};
        let mut secret_bytes: [u8; SECRET_BYTES] = self.secret.expose(|s| {
            let buf = s.encode();
            let mut out = [0u8; SECRET_BYTES];
            out.copy_from_slice(buf.as_ref());
            out
        });
        let mut hex_buf = Zeroizing::new(hex::encode(secret_bytes));
        secret_bytes.zeroize();
        write_mode_0600(path.as_ref(), hex_buf.as_bytes())?;
        hex_buf.zeroize();
        Ok(())
    }
}

#[cfg(all(feature = "plaintext-keys", unix))]
fn write_mode_0600(path: &std::path::Path, data: &[u8]) -> Result<(), Error> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)?;
    Ok(())
}

#[cfg(all(feature = "plaintext-keys", not(unix)))]
fn write_mode_0600(path: &std::path::Path, data: &[u8]) -> Result<(), Error> {
    std::fs::write(path, data)?;
    Ok(())
}

// Custom Debug that does NOT print the secret. Without this the derive
// would expose it through {:?}, which is an audit footgun.
impl core::fmt::Debug for ValidatorBlsKeypair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ValidatorBlsKeypair")
            .field("public", &self.public)
            .field("secret", &"<redacted>")
            .finish()
    }
}

// NOT Default — would yield a predictable secret. Force explicit construction.

#[cfg(test)]
mod tests {
    use super::*;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    #[test]
    fn from_secret_bytes_round_trip() {
        let original = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(7));
        let secret_bytes = original.secret().expose(|s| {
            use commonware_codec::Encode;
            let buf = s.encode();
            let mut out = [0u8; SECRET_BYTES];
            out.copy_from_slice(buf.as_ref());
            out
        });
        let restored = ValidatorBlsKeypair::from_secret_bytes(&secret_bytes).unwrap();
        assert_eq!(original.public_bytes(), restored.public_bytes());
    }

    #[test]
    fn from_secret_bytes_rejects_zero() {
        let zero = [0u8; SECRET_BYTES];
        assert!(matches!(
            ValidatorBlsKeypair::from_secret_bytes(&zero),
            Err(Error::InvalidSecret)
        ));
    }

    #[test]
    fn read_from_file_round_trip_with_and_without_prefix_and_whitespace() {
        let original = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(11));
        let secret_bytes = original.secret().expose(|s| {
            use commonware_codec::Encode;
            let buf = s.encode();
            let mut out = [0u8; SECRET_BYTES];
            out.copy_from_slice(buf.as_ref());
            out
        });
        let hex_bare = hex::encode(secret_bytes);
        let dir = std::env::temp_dir();

        // bare hex
        let path = dir.join(format!("bls_test_bare_{}.key", std::process::id()));
        std::fs::write(&path, &hex_bare).unwrap();
        let kp1 = ValidatorBlsKeypair::read_from_file(&path).unwrap();
        assert_eq!(kp1.public_bytes(), original.public_bytes());

        // 0x-prefixed with trailing newline
        let path2 = dir.join(format!("bls_test_prefixed_{}.key", std::process::id()));
        std::fs::write(&path2, format!("0x{hex_bare}\n")).unwrap();
        let kp2 = ValidatorBlsKeypair::read_from_file(&path2).unwrap();
        assert_eq!(kp2.public_bytes(), original.public_bytes());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path2);
    }

    #[test]
    fn read_from_file_rejects_invalid_hex() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bls_test_invhex_{}.key", std::process::id()));
        std::fs::write(&path, "not valid hex zzz").unwrap();
        assert!(matches!(
            ValidatorBlsKeypair::read_from_file(&path),
            Err(Error::InvalidHex)
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_from_file_rejects_wrong_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bls_test_short_{}.key", std::process::id()));
        std::fs::write(&path, "deadbeef").unwrap();
        assert!(matches!(
            ValidatorBlsKeypair::read_from_file(&path),
            Err(Error::InvalidLength)
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_from_file_missing_path_is_io_error() {
        let path = std::path::PathBuf::from("/this/path/does/not/exist/key");
        assert!(matches!(
            ValidatorBlsKeypair::read_from_file(&path),
            Err(Error::IoRead(_))
        ));
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let kp = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(9));
        let dbg = format!("{kp:?}");
        assert!(dbg.contains("<redacted>"));
        // Sanity: the secret's hex representation must not appear anywhere.
        let secret_hex = kp.secret().expose(|s| {
            use commonware_codec::Encode;
            format!("{:x?}", s.encode())
        });
        assert!(
            !dbg.contains(&secret_hex),
            "Debug output leaked private scalar bytes"
        );
    }
}

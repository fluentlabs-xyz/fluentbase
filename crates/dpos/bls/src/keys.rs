use commonware_codec::EncodeFixed;
use commonware_cryptography::bls12381::primitives::{group::Private, ops};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{
    error::Error,
    secret_store::SecretBackend,
    share_seal::{ShareSealKey, SHARE_AT_REST_INFO},
    BlsPubkey, Variant, PUBKEY_BYTES, SECRET_BYTES,
};

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

    /// Derive the per-epoch DKG-share at-rest seal key (E2): HKDF-SHA256 over the
    /// validator BLS secret as IKM, salted by the chain namespace and
    /// domain-separated by [`SHARE_AT_REST_INFO`]. The raw scalar never crosses
    /// the crate boundary — HKDF runs here over the exposed secret and only the
    /// derived 32-byte [`ShareSealKey`] is returned. Derived ONCE at launch.
    pub fn derive_share_seal_key(&self, chain_id: u64) -> ShareSealKey {
        use commonware_codec::Encode as _;
        use zeroize::Zeroize as _;
        let salt = crate::fluent_namespace(chain_id);
        let mut okm = [0u8; 32];
        self.secret.expose(|s| {
            let mut ikm = Zeroizing::new(s.encode().to_vec());
            let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
            // HKDF-Expand over a 32-byte L cannot fail; the only error case is
            // L > 255*HashLen.
            hk.expand(SHARE_AT_REST_INFO, &mut okm)
                .expect("HKDF-SHA256 expand of 32 bytes is infallible");
            ikm.zeroize();
        });
        let key = ShareSealKey::from_bytes(okm);
        okm.zeroize();
        key
    }

    /// Compressed BLS public key (G2, 96 B for MinSig).
    pub fn public_bytes(&self) -> [u8; PUBKEY_BYTES] {
        // `BlsPubkey::SIZE == PUBKEY_BYTES` for MinSig (G2 compressed);
        // `encode_fixed` asserts the length matches.
        self.public.encode_fixed::<PUBKEY_BYTES>()
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
        Self::from_backend(SecretBackend::Plaintext, path.as_ref())
    }

    /// Load from an EIP-2335 keystore file (version 4).
    pub fn read_from_keystore<P: AsRef<std::path::Path>>(
        path: P,
        password: &[u8],
    ) -> Result<Self, Error> {
        Self::from_backend(SecretBackend::Eip2335 { password }, path.as_ref())
    }

    /// Read raw secret bytes via the at-rest `backend`, then validate the
    /// 32-byte scalar shape at the typed [`Self::from_secret_bytes`] boundary.
    fn from_backend(backend: SecretBackend<'_>, path: &std::path::Path) -> Result<Self, Error> {
        use zeroize::Zeroize as _;
        let bytes = backend.open(path)?;
        let mut arr: [u8; SECRET_BYTES] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidLength)?;
        let result = Self::from_secret_bytes(&arr);
        arr.zeroize();
        result
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
        crate::secret_store::write_mode_0600(path.as_ref(), hex_buf.as_bytes())?;
        hex_buf.zeroize();
        Ok(())
    }
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

    #[test]
    fn derive_share_seal_key_is_deterministic_per_key_and_chain() {
        let kp = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(13));
        let a = kp.derive_share_seal_key(20994);
        let b = kp.derive_share_seal_key(20994);
        assert_eq!(
            a.as_bytes(),
            b.as_bytes(),
            "same key + chain_id derives the same seal key"
        );
        assert_ne!(
            a.as_bytes(),
            kp.derive_share_seal_key(1).as_bytes(),
            "a different chain_id derives a different seal key"
        );
        let other = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(14));
        assert_ne!(
            a.as_bytes(),
            other.derive_share_seal_key(20994).as_bytes(),
            "a different validator key derives a different seal key"
        );
    }

    #[test]
    fn derive_share_seal_key_does_not_return_the_raw_scalar() {
        let kp = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(15));
        let scalar = kp.secret().expose(|s| {
            use commonware_codec::Encode;
            let mut out = [0u8; SECRET_BYTES];
            out.copy_from_slice(s.encode().as_ref());
            out
        });
        assert_ne!(
            kp.derive_share_seal_key(20994).as_bytes(),
            &scalar,
            "derived seal key must not equal the raw BLS scalar (HKDF domain separation)"
        );
    }
}

use commonware_codec::Encode;
use commonware_cryptography::bls12381::primitives::{group::Private, ops};
use rand_core::CryptoRngCore;

use crate::{error::Error, BlsPubkey, PUBKEY_BYTES, SECRET_BYTES, Variant};

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
    /// accessor exists so downstream code in `04_consensus` can pass the
    /// private key into [`crate::scheme::build_signer`] without duplicating it.
    pub(crate) fn secret(&self) -> &Private {
        &self.secret
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
    fn generate_is_deterministic_with_seeded_rng() {
        let mut rng_a = StdRng::seed_from_u64(42);
        let mut rng_b = StdRng::seed_from_u64(42);
        let kp_a = ValidatorBlsKeypair::generate(&mut rng_a);
        let kp_b = ValidatorBlsKeypair::generate(&mut rng_b);
        assert_eq!(kp_a.public_bytes(), kp_b.public_bytes());
    }

    #[test]
    fn distinct_seeds_yield_distinct_keys() {
        let kp_a = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(1));
        let kp_b = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(2));
        assert_ne!(kp_a.public_bytes(), kp_b.public_bytes());
    }

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

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn public_bytes_round_trip(seed in any::<u64>()) {
            use commonware_codec::DecodeExt;
            use rand_core::SeedableRng;
            let kp = ValidatorBlsKeypair::generate(&mut rand_08::rngs::StdRng::seed_from_u64(seed));
            let bytes = kp.public_bytes();
            let decoded = crate::BlsPubkey::decode(bytes.as_slice());
            prop_assert!(decoded.is_ok());
        }
    }
}

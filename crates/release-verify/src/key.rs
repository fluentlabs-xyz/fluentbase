use crate::error::{Result, VerifyError};
use pgp::{
    composed::{Deserializable as _, SignedPublicKey},
    types::{Fingerprint, KeyDetails as _},
};

/// ASCII-armored OpenPGP public key used to authenticate Fluent release artifacts.
pub const FLUENT_RELEASE_PUBKEY_ASC: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----

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
/// Pinning the fingerprint separately from the armored blob means a subtle edit to the key material
/// above cannot go unnoticed: [`ReleaseKey::new`] refuses to build a verifier from a certificate
/// whose fingerprint does not match.
pub const FLUENT_RELEASE_KEY_FINGERPRINT: &str = "0A6D05E5DD98069BA184ED8304A68D620D5208FD";

/// A release certificate that has been parsed, self-checked, and matched against its pin.
///
/// Holding one is the proof that the trust root is the intended one; every verification entry point
/// takes a `ReleaseKey` rather than raw armor so that check cannot be skipped.
#[derive(Debug, Clone)]
pub struct ReleaseKey {
    cert: SignedPublicKey,
}

impl ReleaseKey {
    /// The key Fluent releases are signed with.
    pub fn fluent() -> Result<Self> {
        Self::new(FLUENT_RELEASE_PUBKEY_ASC, FLUENT_RELEASE_KEY_FINGERPRINT)
    }

    /// Parses an armored certificate and checks it against `expected_fingerprint`.
    pub fn new(pubkey_asc: &str, expected_fingerprint: &str) -> Result<Self> {
        let (cert, _headers) = SignedPublicKey::from_string(pubkey_asc)
            .map_err(|err| VerifyError::KeyParse(err.to_string()))?;

        cert.verify_bindings()
            .map_err(|err| VerifyError::KeyBindings(err.to_string()))?;

        let actual = fingerprint_hex(&cert.fingerprint());
        if actual != expected_fingerprint {
            return Err(VerifyError::KeyFingerprint {
                expected: expected_fingerprint.to_owned(),
                actual,
            });
        }

        Ok(Self { cert })
    }

    /// Uppercase hex fingerprint of the primary key.
    pub fn fingerprint(&self) -> String {
        fingerprint_hex(&self.cert.fingerprint())
    }

    pub(crate) fn cert(&self) -> &SignedPublicKey {
        &self.cert
    }
}

pub(crate) fn fingerprint_hex(fingerprint: &Fingerprint) -> String {
    hex::encode_upper(fingerprint.as_bytes())
}

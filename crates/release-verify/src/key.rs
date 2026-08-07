use crate::error::{Result, VerifyError};
use pgp::{
    composed::{Deserializable as _, SignedPublicKey, SignedPublicSubKey},
    packet::{Signature, SignatureType},
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
///
/// The embedded certificate is an offline trust snapshot: this verifier does not query a keyserver.
/// Revocation therefore requires publishing an updated binary containing the revocation or a newly
/// pinned key. When the embedded snapshot contains revocation or expiration metadata, verification
/// enforces it against the current time and refuses revoked or expired signing components.
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
        validate_certificate_policy_at(&cert, pgp::types::Timestamp::now().as_secs() as u64)?;

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

pub(crate) fn validate_certificate_policy_at(cert: &SignedPublicKey, now: u64) -> Result<()> {
    if !cert.details.revocation_signatures.is_empty() {
        return Err(VerifyError::KeyPolicy(
            "the primary release key is revoked".to_owned(),
        ));
    }
    if !primary_can_sign_at(cert, now)
        && !cert
            .public_subkeys
            .iter()
            .any(|subkey| subkey_can_sign_at(subkey, now))
    {
        return Err(VerifyError::KeyPolicy(
            "the certificate has no unrevoked, unexpired signing component".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn primary_can_sign_at(cert: &SignedPublicKey, now: u64) -> bool {
    cert.details
        .direct_signatures
        .iter()
        .chain(
            cert.details
                .users
                .iter()
                .flat_map(|user| user.signatures.iter()),
        )
        .chain(
            cert.details
                .user_attributes
                .iter()
                .flat_map(|attribute| attribute.signatures.iter()),
        )
        .any(|signature| {
            signature.key_flags().sign()
                && component_signature_is_current(
                    signature,
                    cert.primary_key.created_at().as_secs() as u64,
                    now,
                )
        })
}

pub(crate) fn subkey_can_sign_at(subkey: &SignedPublicSubKey, now: u64) -> bool {
    if subkey
        .signatures
        .iter()
        .any(|signature| signature.typ() == Some(SignatureType::SubkeyRevocation))
    {
        return false;
    }
    subkey.signatures.iter().any(|signature| {
        signature.typ() == Some(SignatureType::SubkeyBinding)
            && signature.key_flags().sign()
            && component_signature_is_current(
                signature,
                subkey.key.created_at().as_secs() as u64,
                now,
            )
    })
}

fn component_signature_is_current(signature: &Signature, key_created: u64, now: u64) -> bool {
    let Some(signature_created) = signature.created().map(|created| created.as_secs() as u64)
    else {
        return false;
    };
    time_window_is_current(
        signature_created,
        signature
            .signature_expiration_time()
            .map(|duration| duration.as_secs() as u64),
        key_created,
        signature
            .key_expiration_time()
            .map(|duration| duration.as_secs() as u64),
        now,
    )
}

fn time_window_is_current(
    signature_created: u64,
    signature_lifetime: Option<u64>,
    key_created: u64,
    key_lifetime: Option<u64>,
    now: u64,
) -> bool {
    if signature_created > now {
        return false;
    }
    let expired = |created: u64, lifetime: Option<u64>| {
        lifetime.is_some_and(|lifetime| lifetime != 0 && created.saturating_add(lifetime) <= now)
    };
    !expired(signature_created, signature_lifetime) && !expired(key_created, key_lifetime)
}

pub(crate) fn fingerprint_hex(fingerprint: &Fingerprint) -> String {
    hex::encode_upper(fingerprint.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::time_window_is_current;

    #[test]
    fn component_time_window_rejects_future_and_expired_components() {
        assert!(!time_window_is_current(101, None, 90, None, 100));
        assert!(!time_window_is_current(90, Some(10), 80, None, 100));
        assert!(!time_window_is_current(90, None, 80, Some(20), 100));
        assert!(time_window_is_current(90, Some(0), 80, Some(0), 100));
        assert!(time_window_is_current(90, Some(11), 80, Some(21), 100));
    }
}

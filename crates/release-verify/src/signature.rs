use crate::{
    error::{Result, VerifyError},
    key::{primary_can_sign_at, subkey_can_sign_at, ReleaseKey},
};
use pgp::{
    composed::{Deserializable as _, DetachedSignature, SignedPublicKey, SignedPublicSubKey},
    crypto::hash::HashAlgorithm,
    packet::SignatureType,
    types::{Fingerprint, KeyDetails as _, KeyId},
};
use std::io::Cursor;

/// Digest algorithms accepted on a release signature. SHA-1 and friends are rejected outright.
pub(crate) const ACCEPTED_HASH_ALGORITHMS: &[HashAlgorithm] = &[
    HashAlgorithm::Sha256,
    HashAlgorithm::Sha384,
    HashAlgorithm::Sha512,
    HashAlgorithm::Sha3_256,
    HashAlgorithm::Sha3_512,
];

/// Verifies a detached OpenPGP signature over `data` against the pinned release key.
///
/// Beyond the cryptographic check this rejects signatures that are structurally unusable: text mode
/// signatures (which do not bind the exact bytes), weak digests, and signatures whose issuer is not
/// a signing-capable component of the pinned certificate.
pub fn verify_detached_signature(data: &[u8], armored_sig: &[u8], key: &ReleaseKey) -> Result<()> {
    let (detached, _headers) = DetachedSignature::from_armor_single(Cursor::new(armored_sig))
        .map_err(|err| VerifyError::SignatureParse(err.to_string()))?;
    let sig = &detached.signature;

    match sig.typ() {
        Some(SignatureType::Binary) => {}
        Some(other) => return Err(VerifyError::SignatureType(format!("{other:?}"))),
        None => return Err(VerifyError::SignatureType("unknown".to_owned())),
    }

    let hash_alg = sig
        .hash_alg()
        .ok_or_else(|| VerifyError::WeakDigest("unspecified".to_owned()))?;
    if !ACCEPTED_HASH_ALGORITHMS.contains(&hash_alg) {
        return Err(VerifyError::WeakDigest(format!("{hash_alg:?}")));
    }

    let now = pgp::types::Timestamp::now().as_secs() as u64;
    if let Some(created) = sig.created().map(|created| created.as_secs() as u64) {
        let expired = sig.signature_expiration_time().is_some_and(|lifetime| {
            lifetime.as_secs() != 0 && created.saturating_add(lifetime.as_secs() as u64) <= now
        });
        if created > now || expired {
            return Err(VerifyError::KeyPolicy(
                "the detached signature is not currently valid".to_owned(),
            ));
        }
    } else {
        return Err(VerifyError::KeyPolicy(
            "the detached signature has no creation time".to_owned(),
        ));
    }

    let issuer_fingerprints: Vec<&Fingerprint> = sig.issuer_fingerprint();
    let issuer_key_ids: Vec<&KeyId> = sig.issuer_key_id();
    if issuer_fingerprints.is_empty() && issuer_key_ids.is_empty() {
        return Err(VerifyError::MissingIssuer);
    }

    let candidates = signing_candidates_at(key.cert(), now);
    if candidates.is_empty() {
        return Err(VerifyError::KeyPolicy(
            "the release key has no currently valid signing component".to_owned(),
        ));
    }
    for candidate in candidates {
        let is_issuer = issuer_fingerprints
            .iter()
            .any(|fpr| **fpr == candidate.fingerprint())
            || issuer_key_ids
                .iter()
                .any(|id| **id == candidate.legacy_key_id());
        if !is_issuer {
            continue;
        }
        return candidate
            .verify(&detached, data)
            .map_err(|err| VerifyError::BadSignature(err.to_string()));
    }

    Err(VerifyError::UntrustedIssuer)
}

/// A component of the release certificate that may have produced a data signature.
pub(crate) enum SigningCandidate<'a> {
    Primary(&'a pgp::packet::PublicKey),
    Subkey(&'a SignedPublicSubKey),
}

impl SigningCandidate<'_> {
    pub(crate) fn fingerprint(&self) -> Fingerprint {
        match self {
            Self::Primary(key) => key.fingerprint(),
            Self::Subkey(key) => key.fingerprint(),
        }
    }

    pub(crate) fn legacy_key_id(&self) -> KeyId {
        match self {
            Self::Primary(key) => key.legacy_key_id(),
            Self::Subkey(key) => key.legacy_key_id(),
        }
    }

    fn verify(&self, sig: &DetachedSignature, data: &[u8]) -> pgp::errors::Result<()> {
        match self {
            Self::Primary(key) => sig.verify(*key, data),
            Self::Subkey(key) => sig.verify(*key, data),
        }
    }
}

/// The primary key plus every signing-capable subkey of `cert`.
#[cfg(test)]
pub(crate) fn signing_candidates(cert: &SignedPublicKey) -> Vec<SigningCandidate<'_>> {
    signing_candidates_at(cert, pgp::types::Timestamp::now().as_secs() as u64)
}

fn signing_candidates_at(cert: &SignedPublicKey, now: u64) -> Vec<SigningCandidate<'_>> {
    let mut candidates = Vec::new();
    if primary_can_sign_at(cert, now) {
        candidates.push(SigningCandidate::Primary(&cert.primary_key));
    }
    candidates.extend(
        cert.public_subkeys
            .iter()
            .filter(|subkey| subkey_can_sign_at(subkey, now))
            .map(SigningCandidate::Subkey),
    );
    candidates
}

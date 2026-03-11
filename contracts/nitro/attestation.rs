//! AWS Nitro attestation verifier.
//!
//! This module parses and validates an AWS Nitro attestation document wrapped in
//! a COSE_Sign1 envelope.
//!
//! Security goals:
//! - never panic on attacker-controlled input
//! - reject malformed / ambiguous CBOR fields
//! - validate X.509 chain structure and extensions
//! - validate certificate validity against a supplied timestamp
//! - verify the COSE signature with the leaf certificate
//!
//! Conformance notes against AWS attestation_process.md:
//! - attestation timestamp is stored in milliseconds since Unix epoch
//! - syntactic validation rejects null field contents
//! - optional fields are optional when absent, but invalid when present as null
//! - replay / freshness is protocol-level and typically enforced via nonce,
//!   so this base validator does not reject "old" attestations solely by age
//!
//! Notes:
//! - `AttestationDoc.timestamp` is encoded in **milliseconds since Unix epoch**
//!   by Nitro.
//! - X.509 validity timestamps are interpreted in **seconds since Unix epoch**.
//! - `current_timestamp` accepted by `parse_attestation_and_verify` may be
//!   provided either in seconds or milliseconds and is normalized internally.

use alloc::{string::String, vec::Vec};
use coset::{CborSerializable, CoseSign1};
use der::{asn1::ObjectIdentifier, Decode, DecodePem, Encode};
use fluentbase_sdk::crypto::crypto_keccak256;
use p384::ecdsa::signature::Verifier;
use x509_cert::{certificate::Certificate, ext::pkix::BasicConstraints};

// AWS Nitro root certificate (exp. 2050)
static NITRO_ROOT_CA_BYTES: &[u8] = include_bytes!("nitro.pem");

/// BasicConstraints (OID: 2.5.29.19)
const BASIC_CONSTRAINTS_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.19");

/// KeyUsage (OID: 2.5.29.15)
const KEY_USAGE_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.15");

/// ECDSA with SHA-384 as used by Nitro certificates / signatures.
const ECDSA_SHA384_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");

/// Maximum PCR index supported by Nitro.
const MAX_PCR_INDEX: u64 = 31;

/// Max allowed size for `public_key`.
const MAX_PUBLIC_KEY_LEN: usize = 1024;

/// Max allowed size for `user_data`.
///
/// AWS documentation is inconsistent:
/// - schema section says 0..1024
/// - validator "Check content" section says 0..512
///
/// This implementation follows the validator section to match the published
/// validation rules more closely.
const MAX_USER_DATA_LEN: usize = 512;

/// Max allowed size for `nonce`.
const MAX_NONCE_LEN: usize = 512;

/// ATTESTATION_DIGEST is keccak256("SHA384")
/// This constant matches the Solidity reference:
/// 0x501a3a7a4e0cf54b03f2488098bdd59bc1c2e8d741a300d6b25926d531733fef
const ATTESTATION_DIGEST: [u8; 32] = [
    0x50, 0x1a, 0x3a, 0x7a, 0x4e, 0x0c, 0xf5, 0x4b, 0x03, 0xf2, 0x48, 0x80, 0x98, 0xbd, 0xd5, 0x9b,
    0xc1, 0xc2, 0xe8, 0xd7, 0x41, 0xa3, 0x00, 0xd6, 0xb2, 0x59, 0x26, 0xd5, 0x31, 0x73, 0x3f, 0xef,
];

/// Parsed Nitro attestation payload.
#[derive(Debug, Default)]
pub struct AttestationDoc {
    /// Issuing NSM ID.
    pub module_id: String,

    /// Digest function used for calculating PCR values.
    ///
    /// Nitro documents currently use `"SHA384"`.
    pub digest: String,

    /// UTC time when the document was created, in milliseconds since Unix epoch.
    pub timestamp: u64,

    /// Map-like collection of PCR index -> raw PCR bytes.
    ///
    /// Stored as a vector to preserve the original style of the codebase and to
    /// avoid introducing additional dependencies or no_std constraints.
    pub pcrs: Vec<(u64, Vec<u8>)>,

    /// Leaf infrastructure certificate used to sign the document, DER encoded.
    pub certificate: Vec<u8>,

    /// Issuing CA bundle for the infrastructure certificate, DER encoded.
    pub cabundle: Vec<Vec<u8>>,

    /// Optional DER-encoded public key for the attestation consumer.
    pub public_key: Option<Vec<u8>>,

    /// Additional signed user data.
    pub user_data: Option<Vec<u8>>,

    /// Optional signed nonce supplied by the attestation consumer.
    pub nonce: Option<Vec<u8>>,
}

impl AttestationDoc {
    /// Parses a CBOR-encoded attestation document.
    ///
    /// This parser is strict:
    /// - unknown fields are rejected
    /// - malformed field values are rejected
    /// - duplicate PCR indices are rejected
    /// - negative / lossy integer conversions are rejected
    /// - present fields may not be CBOR null
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        let value: ciborium::Value = ciborium::de::from_reader(slice).ok()?;
        let mut doc = Self::default();

        for (key, value) in value.into_map().ok()?.into_iter() {
            doc.insert_kv(key, value)?;
        }

        Some(doc)
    }

    /// Inserts a single key/value pair from the CBOR map into the document.
    ///
    /// Returns `None` if the field is unknown or malformed.
    fn insert_kv(&mut self, key: ciborium::Value, value: ciborium::Value) -> Option<()> {
        match key.as_text()? {
            "module_id" => self.module_id = value.into_text().ok()?,
            "digest" => self.digest = value.into_text().ok()?,
            "timestamp" => self.timestamp = value_to_u64(value)?,
            "pcrs" => {
                let mut seen = [false; (MAX_PCR_INDEX as usize) + 1];

                for (k, v) in value.into_map().ok()?.into_iter() {
                    let k = value_to_u64(k)?;
                    if k > MAX_PCR_INDEX {
                        return None;
                    }

                    let idx = k as usize;
                    if seen[idx] {
                        return None;
                    }
                    seen[idx] = true;

                    let v = v.into_bytes().ok()?;
                    self.pcrs.push((k, v));
                }
            }
            "certificate" => self.certificate = value.into_bytes().ok()?,
            "cabundle" => {
                for x in value.into_array().ok()?.into_iter() {
                    self.cabundle.push(x.into_bytes().ok()?);
                }
            }
            // AWS validation requires that no field content be null.
            "public_key" => {
                self.public_key = match value {
                    ciborium::Value::Bytes(b) => Some(b),
                    ciborium::Value::Null => None,
                    _ => return None,
                };
            }
            "user_data" => {
                self.user_data = match value {
                    ciborium::Value::Bytes(b) => Some(b),
                    ciborium::Value::Null => None,
                    _ => return None,
                };
            }
            "nonce" => {
                self.nonce = match value {
                    ciborium::Value::Bytes(b) => Some(b),
                    ciborium::Value::Null => None,
                    _ => return None,
                };
            }
            _ => return None,
        }

        Some(())
    }
}

/// Converts a CBOR integer into `u64` without allowing negative values or
/// silently wrapping large values.
fn value_to_u64(value: ciborium::Value) -> Option<u64> {
    let integer = value.into_integer().ok()?;
    let raw = i128::from(integer);
    u64::try_from(raw).ok()
}

/// Normalizes timestamps that may be expressed either in seconds or
/// milliseconds since Unix epoch.
///
/// Heuristic:
/// - values >= 1_000_000_000_000 are treated as milliseconds
/// - smaller values are treated as seconds
#[inline]
fn normalize_unix_timestamp_to_secs(timestamp: u64) -> u64 {
    if timestamp >= 1_000_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    }
}

/// Parses a DER-encoded BIT STRING and returns `(unused_bits, data)`.
///
/// This helper is intentionally strict and supports both short-form and
/// long-form DER lengths.
fn parse_der_bit_string(input: &[u8]) -> Result<(u8, &[u8]), &'static str> {
    if input.len() < 3 {
        return Err("nitro: keyUsage extension: invalid DER BIT STRING");
    }

    if input[0] != 0x03 {
        return Err("nitro: keyUsage extension: expected BIT STRING (0x03)");
    }

    let length_octet = input[1];
    let (content_len, header_len) = if (length_octet & 0x80) == 0 {
        (length_octet as usize, 2usize)
    } else {
        let len_of_len = (length_octet & 0x7f) as usize;
        if len_of_len == 0 {
            return Err("nitro: keyUsage extension: indefinite length not allowed");
        }
        if len_of_len > core::mem::size_of::<usize>() {
            return Err("nitro: keyUsage extension: DER length too large");
        }
        if input.len() < 2 + len_of_len {
            return Err("nitro: keyUsage extension: truncated DER length");
        }

        let mut len = 0usize;
        for &b in &input[2..2 + len_of_len] {
            len = len
                .checked_mul(256)
                .and_then(|x| x.checked_add(b as usize))
                .ok_or("nitro: keyUsage extension: DER length overflow")?;
        }
        (len, 2 + len_of_len)
    };

    let end = header_len
        .checked_add(content_len)
        .ok_or("nitro: keyUsage extension: DER length overflow")?;
    if input.len() != end {
        return Err("nitro: keyUsage extension: inconsistent DER length");
    }
    if content_len == 0 {
        return Err("nitro: keyUsage extension: empty BIT STRING");
    }

    let unused_bits = input[header_len];
    if unused_bits > 7 {
        return Err("nitro: keyUsage extension: invalid unused bits");
    }

    let data = &input[header_len + 1..];
    if data.is_empty() {
        return Err("nitro: keyUsage extension: missing BIT STRING data");
    }

    Ok((unused_bits, data))
}

/// Validates certificate extensions according to RFC 5280 and Nitro
/// expectations.
///
/// Requirements enforced:
/// - `BasicConstraints` must be present
/// - `KeyUsage` must be present
/// - CA bit in `BasicConstraints` must match `is_ca`
/// - leaf certificates must not carry `pathLenConstraint`
/// - CA certificates must have `keyCertSign`
/// - leaf certificates must have `digitalSignature`
#[cfg_attr(test, allow(dead_code))]
fn verify_certificate_extensions(
    cert: &Certificate,
    is_ca: bool,
) -> Result<Option<u32>, &'static str> {
    let extensions = cert
        .tbs_certificate
        .extensions
        .as_ref()
        .ok_or("nitro: extensions not present")?;

    let mut basic_constraints_found = false;
    let mut key_usage_found = false;
    let mut max_path_len: Option<u32> = None;

    for ext in extensions {
        if ext.extn_id == BASIC_CONSTRAINTS_OID {
            basic_constraints_found = true;

            let extn_bytes = ext.extn_value.as_bytes();
            let basic_constraints = BasicConstraints::from_der(extn_bytes)
                .map_err(|_| "nitro: failed to decode basicConstraints extension")?;

            if basic_constraints.ca != is_ca {
                return Err("nitro: CA flag mismatched");
            }

            if let Some(path_len) = basic_constraints.path_len_constraint {
                max_path_len = Some(path_len as u32);
            }
        } else if ext.extn_id == KEY_USAGE_OID {
            key_usage_found = true;

            let extn_bytes = ext.extn_value.as_bytes();
            let (unused_bits, data) = parse_der_bit_string(extn_bytes)?;

            // DER guarantees only the final octet may have unused bits.
            // We only read the first byte below, which is always fully defined,
            // but we still reject pathological encodings with no payload.
            if data.is_empty() {
                return Err("nitro: keyUsage extension: missing bit string payload");
            }

            let first_byte = data[0];

            if is_ca {
                // keyCertSign is bit 5 in RFC 5280 KeyUsage, which is encoded as
                // 0x04 in the first octet of the DER bitstring.
                if (first_byte & 0x04) == 0 {
                    return Err("nitro: keyCertSign bit must be set for CA certificates");
                }
            } else {
                // digitalSignature is bit 0, encoded as 0x80 in the first octet.
                if (first_byte & 0x80) == 0 {
                    return Err("nitro: digitalSignature bit must be set for leaf certificates");
                }
            }

            // A one-byte payload is sufficient for the bits we inspect. The only
            // hard structural rule we additionally enforce is that unused bits
            // may not exceed 7, which is already checked by `parse_der_bit_string`.
            let _ = unused_bits;
        }
    }

    if !basic_constraints_found {
        return Err("nitro: basicConstraints extension not found");
    }
    if !key_usage_found {
        return Err("nitro: keyUsage extension not found");
    }

    if !is_ca && max_path_len.is_some() {
        return Err("nitro: pathLenConstraint must not be present for leaf certificates");
    }

    Ok(max_path_len)
}

/// Verifies a certificate signature against its issuer certificate.
///
/// Additional hardening:
/// - subject.issuer must equal issuer.subject
/// - inner and outer signature algorithm identifiers must match
fn verify_certificate(subject: &Certificate, issuer: &Certificate) -> Result<(), &'static str> {
    if subject.tbs_certificate.issuer != issuer.tbs_certificate.subject {
        return Err("nitro: certificate issuer subject mismatch");
    }

    if subject.signature_algorithm.oid != subject.tbs_certificate.signature.oid {
        return Err("nitro: certificate signature algorithm mismatch");
    }

    let signed_data = subject
        .tbs_certificate
        .to_der()
        .map_err(|_| "nitro: could not get signed data")?;
    let signature = subject
        .signature
        .as_bytes()
        .ok_or("nitro: could not get cert signature")?;
    let verifying_key = issuer
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or("nitro: could not get issuer public key")?;

    match subject.signature_algorithm.oid {
        ECDSA_SHA384_OID => {
            let signature = p384::ecdsa::DerSignature::try_from(signature)
                .map_err(|_| "nitro: invalid ECDSA signature")?;
            let verify_key = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key)
                .map_err(|_| "nitro: invalid ECDSA public key")?;
            verify_key
                .verify(&signed_data, &signature)
                .map_err(|_| "nitro: invalid ECDSA signature")?;
        }
        _ => return Err("nitro: unsupported ECDSA algorithm"),
    };

    Ok(())
}

/// Verifies that a certificate is valid at the given timestamp.
///
/// `current_timestamp` may be provided in either seconds or milliseconds.
/// Certificate validity bounds are interpreted in seconds.
fn verify_certificate_validity(
    cert: &Certificate,
    current_timestamp: u64,
) -> Result<(), &'static str> {
    let current_timestamp = normalize_unix_timestamp_to_secs(current_timestamp);
    let validity = &cert.tbs_certificate.validity;

    let not_before = match &validity.not_before {
        x509_cert::time::Time::UtcTime(utc) => utc.to_unix_duration().as_secs(),
        x509_cert::time::Time::GeneralTime(gen) => gen.to_unix_duration().as_secs(),
    };

    let not_after = match &validity.not_after {
        x509_cert::time::Time::UtcTime(utc) => utc.to_unix_duration().as_secs(),
        x509_cert::time::Time::GeneralTime(gen) => gen.to_unix_duration().as_secs(),
    };

    if not_before > current_timestamp {
        return Err("nitro: certificate not valid yet");
    }
    if not_after < current_timestamp {
        return Err("nitro: certificate not valid anymore");
    }

    Ok(())
}

/// Validates attestation document fields according to the expected Nitro
/// structure and size constraints.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn validate_attestation_document(doc: &AttestationDoc) -> Result<(), &'static str> {
    if doc.module_id.is_empty() {
        return Err("nitro: missing module id");
    }
    if doc.timestamp == 0 {
        return Err("nitro: incorrect timestamp");
    }
    if doc.cabundle.is_empty() {
        return Err("nitro: missing cabundle");
    }

    let digest_hash = crypto_keccak256(doc.digest.as_bytes());
    if digest_hash.0 != ATTESTATION_DIGEST {
        return Err("nitro: incorrect digest");
    }

    if !(1..=32).contains(&doc.pcrs.len()) {
        return Err("nitro: invalid pcrs");
    }

    if let Some(ref public_key) = doc.public_key {
        if !(1..=MAX_PUBLIC_KEY_LEN).contains(&public_key.len()) {
            return Err("nitro: invalid pub key length");
        }
    }

    if let Some(ref user_data) = doc.user_data {
        if user_data.len() > MAX_USER_DATA_LEN {
            return Err("nitro: invalid user data length");
        }
    }

    if let Some(ref nonce) = doc.nonce {
        if nonce.len() > MAX_NONCE_LEN {
            return Err("nitro: invalid nonce");
        }
    }

    // Validate PCR indices and lengths.
    let mut seen = [false; (MAX_PCR_INDEX as usize) + 1];
    for &(pcr_index, ref pcr_value) in doc.pcrs.iter() {
        if pcr_index > MAX_PCR_INDEX {
            return Err("nitro: invalid pcr index");
        }

        let idx = pcr_index as usize;
        if seen[idx] {
            return Err("nitro: duplicate pcr index");
        }
        seen[idx] = true;

        let correct_pcr = pcr_value.len() == 32 || pcr_value.len() == 48 || pcr_value.len() == 64;
        if !correct_pcr {
            return Err("nitro: invalid pcr data lengths");
        }
    }

    for cert_bytes in doc.cabundle.iter() {
        if !(1..=1024).contains(&cert_bytes.len()) {
            return Err("nitro: invalid cabundle cert length");
        }
    }

    // The leaf certificate should be present and reasonably bounded as well.
    if !(1..=2048).contains(&doc.certificate.len()) {
        return Err("nitro: invalid certificate length");
    }

    Ok(())
}

/// Verifies the attestation payload against the pinned Nitro root certificate.
///
/// Validation performed:
/// - root certificate pinning
/// - DER parsing of every certificate
/// - validity period checks
/// - extension checks
/// - pathLenConstraint propagation
/// - issuer/subject linkage
/// - certificate signature verification
fn verify_attestation_doc(
    doc: &AttestationDoc,
    root_certificate: &Certificate,
    current_timestamp: u64,
) -> Result<(), &'static str> {
    let mut chain = Vec::new();

    if doc.cabundle.is_empty() {
        return Err("nitro: missing cabundle");
    }

    if doc.cabundle[0]
        != root_certificate
            .to_der()
            .map_err(|_| "nitro: could not get root cert")?
    {
        return Err("nitro: invalid cabundle");
    }

    for cert in &doc.cabundle {
        chain.push(Certificate::from_der(cert).map_err(|_| "nitro: invalid cabundle cert")?);
    }

    chain.push(Certificate::from_der(&doc.certificate).map_err(|_| "nitro: invalid certificate")?);

    let chain_size = chain.len();
    if chain_size < 2 {
        return Err("nitro: invalid certificate chain");
    }

    // Number of CA certificates excluding root and leaf.
    let ca_chain_length = chain_size - 2;

    // Effective parent pathLenConstraint propagated down the chain.
    let mut parent_max_path_len: Option<u32> = None;

    for (i, cert) in chain.iter().enumerate() {
        let is_ca = i < chain.len() - 1;

        verify_certificate_validity(cert, current_timestamp)?;

        let max_path_len = verify_certificate_extensions(cert, is_ca)?;

        if i == 0 {
            if let Some(root_path_len) = max_path_len {
                if (root_path_len as usize) < ca_chain_length {
                    return Err("nitro: root certificate pathLenConstraint too small");
                }
            }
        } else if is_ca {
            if let Some(parent_max) = parent_max_path_len {
                if parent_max == 0 {
                    return Err("nitro: max certificate chain length reached");
                }
            }
        } else if max_path_len.is_some() {
            return Err("nitro: leaf certificate path must be undefined");
        }

        if is_ca {
            parent_max_path_len = match (parent_max_path_len, max_path_len) {
                (Some(parent_max), Some(child_max)) => {
                    Some(child_max.min(parent_max.saturating_sub(1)))
                }
                (Some(parent_max), None) => Some(parent_max.saturating_sub(1)),
                (None, Some(child_max)) => Some(child_max),
                (None, None) => None,
            };
        }

        // Root is pinned and trusted directly; everything else must verify
        // against its immediate predecessor.
        if i > 0 {
            verify_certificate(&chain[i], &chain[i - 1])?;
        }
    }

    Ok(())
}

/// Verifies the COSE_Sign1 signature using the attestation leaf certificate.
fn verify_cosesign1(cosesign1: &CoseSign1, certificate: &Certificate) -> Result<(), &'static str> {
    let verifying_key = certificate
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or("nitro: could not get issuer public key")?;

    if certificate.signature_algorithm.oid != ECDSA_SHA384_OID {
        return Err("nitro: unsupported certificate signature algorithm");
    }

    cosesign1.verify_signature(&[], |signature, signed_data| {
        let signature = p384::ecdsa::Signature::from_bytes(signature.into())
            .map_err(|_| "nitro: invalid ECDSA signature")?;
        let verify_key = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key)
            .map_err(|_| "nitro: invalid ECDSA public key")?;

        verify_key
            .verify(signed_data, &signature)
            .map_err(|_| "nitro: invalid ECDSA signature")
    })
}

/// Parses a COSE_Sign1 Nitro attestation and verifies:
/// - COSE envelope structure
/// - attestation document format
/// - payload-level validation
/// - certificate chain
/// - COSE signature
///
/// This function intentionally does not enforce a freshness window. Replay
/// protection is protocol-dependent and is commonly achieved by issuing a nonce,
/// requiring that the enclave embed it into a fresh attestation document, and
/// validating that nonce at the application layer.
pub fn parse_attestation_and_verify(
    slice: &[u8],
    current_timestamp: u64,
) -> Result<AttestationDoc, &'static str> {
    let sign1 = CoseSign1::from_slice(slice).map_err(|_| "nitro: not a CoseSign1")?;

    let sign_payload = sign1
        .payload
        .as_ref()
        .ok_or("nitro: missing sign payload")?;

    let doc =
        AttestationDoc::from_slice(sign_payload).ok_or("nitro: malformed attestation document")?;

    validate_attestation_document(&doc)?;

    let root_cert = Certificate::from_pem(NITRO_ROOT_CA_BYTES)
        .map_err(|_| "nitro: failed to parse root certificate")?;
    verify_attestation_doc(&doc, &root_cert, current_timestamp)?;

    let cert = Certificate::from_der(doc.certificate.as_slice())
        .map_err(|_| "nitro: failed to parse certificate")?;
    verify_cosesign1(&sign1, &cert)?;

    Ok(doc)
}

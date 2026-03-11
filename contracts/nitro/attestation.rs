use alloc::{string::String, vec::Vec};
use coset::{CborSerializable, CoseSign1};
use der::{asn1::ObjectIdentifier, Decode, DecodePem, Encode};
use fluentbase_sdk::crypto::crypto_keccak256;
use p384::ecdsa::signature::Verifier;
use x509_cert::{certificate::Certificate, ext::pkix::BasicConstraints};

// AWS Nitro root certificate (exp. 2050)
static NITRO_ROOT_CA_BYTES: &[u8] = include_bytes!("nitro.pem");

// Standard X.509 certificate extension Object Identifiers (OIDs) as defined in RFC 5280
// These are globally unique identifiers for certificate extensions
//
// BasicConstraints (OID: 2.5.29.19)
// - Indicates whether the certificate is for a Certificate Authority (CA)
// - May include pathLenConstraint to limit certification path depth
// - Defined in RFC 5280 Section 4.2.1.9
const BASIC_CONSTRAINTS_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.19");

// KeyUsage (OID: 2.5.29.15)
// - Specifies the purpose of the key contained in the certificate
// - Defines which cryptographic operations the key can be used for
//   (e.g., digitalSignature, keyCertSign for CA certificates)
// - Defined in RFC 5280 Section 4.2.1.3
const KEY_USAGE_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.15");

// ATTESTATION_DIGEST is keccak256("SHA384")
// This constant matches the Solidity reference: 0x501a3a7a4e0cf54b03f2488098bdd59bc1c2e8d741a300d6b25926d531733fef
const ATTESTATION_DIGEST: [u8; 32] = [
    0x50, 0x1a, 0x3a, 0x7a, 0x4e, 0x0c, 0xf5, 0x4b, 0x03, 0xf2, 0x48, 0x80, 0x98, 0xbd, 0xd5, 0x9b,
    0xc1, 0xc2, 0xe8, 0xd7, 0x41, 0xa3, 0x00, 0xd6, 0xb2, 0x59, 0x26, 0xd5, 0x31, 0x73, 0x3f, 0xef,
];

#[derive(Debug, Default)]
pub struct AttestationDoc {
    /// Issuing NSM ID
    pub module_id: String,

    /// The digest function used for calculating the register values
    /// Can be: "SHA256" | "SHA512"
    pub digest: String,

    /// UTC time when document was created expressed as milliseconds since Unix Epoch
    pub timestamp: u64,

    /// Map of all locked PCRs at the moment the attestation document was generated
    pub pcrs: Vec<(u64, Vec<u8>)>,

    /// The infrastucture certificate used to sign the document, DER encoded
    pub certificate: Vec<u8>,
    /// Issuing CA bundle for infrastructure certificate
    pub cabundle: Vec<Vec<u8>>,

    /// An optional DER-encoded key the attestation consumer can use to encrypt data with
    pub public_key: Option<Vec<u8>>,

    /// Additional signed user data, as defined by protocol.
    pub user_data: Option<Vec<u8>>,

    /// An optional cryptographic nonce provided by the attestation consumer as a proof of
    /// authenticity.
    pub nonce: Option<Vec<u8>>,
}

impl AttestationDoc {
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        let value: ciborium::Value = ciborium::de::from_reader(slice).ok()?;
        let mut doc = Self::default();
        for (key, value) in value.into_map().ok()?.into_iter() {
            doc.insert_kv(key, value)?;
        }
        Some(doc)
    }

    fn insert_kv(&mut self, key: ciborium::Value, value: ciborium::Value) -> Option<()> {
        match key.as_text()? {
            "module_id" => self.module_id = value.into_text().ok()?,
            "digest" => self.digest = value.into_text().ok()?,
            "timestamp" => self.timestamp = i128::from(value.into_integer().ok()?) as u64,
            "pcrs" => {
                for (k, v) in value.into_map().ok()?.into_iter() {
                    let k = i128::from(k.into_integer().ok()?) as u64;
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

/// Validates certificate extensions according to RFC 5280 and Nitro specification.
/// Both BasicConstraints and KeyUsage extensions must be present.
/// Returns the pathLenConstraint if present (None means unlimited).
/// Note: Critical flag is not enforced, aligning with reference:
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
            // BasicConstraints extension found (critical flag is optional)
            basic_constraints_found = true;

            // Decode BasicConstraints from the extension value (which is an OctetString)
            let extn_bytes = ext.extn_value.as_bytes();
            let Ok(basic_constraints) = BasicConstraints::from_der(extn_bytes) else {
                return Err("nitro: failed to decode basicConstraints extension");
            };

            // Verify CA flag matches expected value
            if basic_constraints.ca != is_ca {
                return Err("nitro: CA flag mismatched");
            }

            // Extract pathLenConstraint if present
            if let Some(path_len) = basic_constraints.path_len_constraint {
                max_path_len = Some(path_len as u32);
            }
        } else if ext.extn_id == KEY_USAGE_OID {
            // KeyUsage extension found (a critical flag is optional)
            key_usage_found = true;

            // Decode KeyUsage as BitString from the extension value
            // The extension value is an OctetString containing a DER-encoded BitString
            let extn_bytes = ext.extn_value.as_bytes();

            // Parse the BitString manually (similar to Solidity code)
            // BitString DER format: [0x03] [length] [unused_bits: u8] [data...]
            // We expect tag 0x03 (BIT STRING), unused_bits should be 0x00 (full bytes)
            if extn_bytes.is_empty() {
                return Err("nitro: keyUsage extension: value is empty");
            }

            // Verify it's a BIT STRING (tag 0x03)
            if extn_bytes[0] != 0x03 {
                return Err("nitro: keyUsage extension: expected BIT STRING (0x03)");
            }

            // Parse length (simple DER length encoding)
            let mut data_start = 2; // Skip tag (1 byte) and length (1 byte for short form)
            if extn_bytes.len() < data_start + 1 {
                return Err("nitro: keyUsage extension: invalid length");
            }

            // Check unused bits (used bits only affect the last byte)
            let _unused_bits = extn_bytes[data_start];

            // Skip unused_bits byte to get the actual data
            data_start += 1;
            if extn_bytes.len() <= data_start {
                return Err("nitro: keyUsage extension: invalid length");
            }

            // Get the first byte of KeyUsage data
            // Unused bits only affect interpretation of the last byte, so the first byte is always valid
            let first_byte = extn_bytes[data_start];

            // Ensure we have at least one full byte of data (unused bits only affect the last byte)
            // For our checks (bit 0 and bit 5), we only need the first byte, so we're good

            if is_ca {
                // For CA certificates: keyCertSign (bit 5) must be set
                // In the bitstring, bit 5 means the 6th bit from the left (0-indexed: bit 5)
                // This corresponds to: 0x04 (binary: 00000100)
                if (first_byte & 0x04) == 0 {
                    return Err("nitro: keyCertSign bit must be set for CA certificates");
                }
            } else {
                // For leaf certificates: digitalSignature (bit 0) must be set
                // Bit 0 is the leftmost bit, which corresponds to: 0x80 (binary: 10000000)
                if (first_byte & 0x80) == 0 {
                    return Err("nitro: digitalSignature bit must be set for leaf certificates");
                }
            }
        }
    }

    if !basic_constraints_found {
        return Err("nitro: basicConstraints extension not found");
    } else if !key_usage_found {
        return Err("nitro: keyUsage extension not found");
    }

    // For leaf certificates, pathLenConstraint must not be present
    if !is_ca && max_path_len.is_some() {
        return Err("nitro: pathLenConstraint must not be present for leaf certificates");
    }

    Ok(max_path_len)
}

fn verify_certificate(subject: &Certificate, issuer: &Certificate) -> Result<(), &'static str> {
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
        ecdsa::ECDSA_SHA384_OID => {
            let Ok(signature) = p384::ecdsa::DerSignature::try_from(signature) else {
                return Err("nitro: invalid ECDSA signature");
            };
            let Ok(verify_key) = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key) else {
                return Err("nitro: invalid ECDSA public key");
            };
            verify_key
                .verify(&signed_data, &signature)
                .map_err(|_| "nitro: invalid ECDSA signature")?;
        }
        _ => {
            return Err("nitro: unsupported ECDSA algorithm");
        }
    };

    Ok(())
}

/// Verifies that a certificate is valid at the given timestamp.
/// Checks that notBefore <= timestamp <= notAfter.
fn verify_certificate_validity(
    cert: &Certificate,
    current_timestamp: u64,
) -> Result<(), &'static str> {
    let validity = &cert.tbs_certificate.validity;

    // Convert certificate validity times to Unix timestamps
    // x509-cert uses Time which can be UTCTime or GeneralizedTime
    // Both have represented time as seconds since Unix epoch
    let not_before = match &validity.not_before {
        x509_cert::time::Time::UtcTime(utc) => {
            // UTCTime is seconds since 1970-01-01 00:00:00 UTC
            utc.to_unix_duration().as_secs()
        }
        x509_cert::time::Time::GeneralTime(gen) => {
            // GeneralTime is also seconds since 1970-01-01 00:00:00 UTC
            gen.to_unix_duration().as_secs()
        }
    };

    let not_after = match &validity.not_after {
        x509_cert::time::Time::UtcTime(utc) => utc.to_unix_duration().as_secs(),
        x509_cert::time::Time::GeneralTime(gen) => gen.to_unix_duration().as_secs(),
    };

    if not_before > current_timestamp {
        return Err("nitro: certificate not valid yet");
    } else if not_after < current_timestamp {
        return Err("nitro: certificate not valid anymore");
    }

    Ok(())
}

/// Validates attestation document fields according to the specification.
/// This implements the validation requirements from validateAttestation in the Solidity reference.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn validate_attestation_document(doc: &AttestationDoc) -> Result<(), &'static str> {
    if doc.module_id.is_empty() {
        return Err("nitro: missing module id");
    } else if doc.timestamp == 0 {
        return Err("nitro: incorrect timestamp");
    } else if doc.cabundle.is_empty() {
        return Err("nitro: missing cabundle");
    }

    // require(attestationTbs.keccak(ptrs.digest) == ATTESTATION_DIGEST, "invalid digest");
    let digest_hash = crypto_keccak256(doc.digest.as_bytes());
    if digest_hash.0 != ATTESTATION_DIGEST {
        return Err("nitro: incorrect digest");
    }

    // require(1 <= ptrs.pcrs.length && ptrs.pcrs.length <= 32, "invalid pcrs");
    if !(1..=32).contains(&doc.pcrs.len()) {
        return Err("nitro: invalid pcrs");
    }

    // require(ptrs.publicKey.isNull() || (1 <= ptrs.publicKey.length() && ptrs.publicKey.length() <= 1024), "invalid pub key");
    if let Some(ref public_key) = doc.public_key {
        if !(1..=1024).contains(&public_key.len()) {
            return Err("nitro: invalid pub key length");
        }
    }

    // require(ptrs.userData.isNull() || (ptrs.userData.length() <= 512), "invalid user data");
    if let Some(ref user_data) = doc.user_data {
        if user_data.len() > 512 {
            return Err("nitro: invalid user data length");
        }
    }

    // require(ptrs.nonce.isNull() || (ptrs.nonce.length() <= 512), "invalid nonce");
    if let Some(ref nonce) = doc.nonce {
        if nonce.len() > 512 {
            return Err("nitro: invalid nonce");
        }
    }

    // Validate each PCR length
    // require(ptrs.pcrs[i].length() == 32 || ptrs.pcrs[i].length() == 48 || ptrs.pcrs[i].length() == 64, "invalid pcr");
    for (_, pcr_value) in doc.pcrs.iter() {
        let correct_pcr = pcr_value.len() == 32 || pcr_value.len() == 48 || pcr_value.len() == 64;
        if !correct_pcr {
            return Err("nitro: invalid pcr data lengths");
        }
    }

    // Validate each cabundle certificate length
    // require(1 <= ptrs.cabundle[i].length() && ptrs.cabundle[i].length() <= 1024, "invalid cabundle cert");
    for cert_bytes in doc.cabundle.iter() {
        if !(1..=1024).contains(&cert_bytes.len()) {
            return Err("nitro: invalid cabundle cert length");
        }
    }
    Ok(())
}

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
    // Chain size excluding root and leaf = number of intermediate CA certificates
    // The root is at index 0, leaf is at index chain.len() - 1
    let ca_chain_length = chain_size - 2; // Excluding root (index 0) and leaf (last index)

    // Track parent maxPathLen for pathLenConstraint validation
    let mut parent_max_path_len: Option<u32> = None;

    // Verify extensions, validity, and signatures for each certificate in the chain
    for (i, cert) in chain.iter().enumerate() {
        let is_ca = i < chain.len() - 1; // All certs except the last are CA certs

        // Verify certificate validity period (Section 3.2.3.1)
        verify_certificate_validity(cert, current_timestamp)?;

        let max_path_len = verify_certificate_extensions(cert, is_ca)?;

        if i == 0 {
            // Root certificate (index 0) specific validation
            // pathLenConstraint must be >= chain size (number of CA certs in chain excluding root)
            // If pathLenConstraint is undefined (None), it's unlimited, which is valid
            if let Some(root_path_len) = max_path_len {
                if !(root_path_len as usize >= ca_chain_length) {
                    return Err("nitro: root certificate pathLenConstraint too small");
                }
            }
            // Root certificate should have keyCertSign (already validated in verify_certificate_extensions)
        } else if is_ca {
            // Intermediate CA certificates (all except root and leaf)
            // Check that pathLenConstraint is not exceeded
            if let Some(parent_max) = parent_max_path_len {
                // Parent's pathLenConstraint limits how many more CA certs can follow
                // If parent_max is 0, no more certificates can follow (a chain already too long)
                if parent_max == 0 {
                    return Err("nitro: max certificate chain length reached");
                }
                // Note: a child's pathLenConstraint can be greater than its parent's.
                // We enforce the parent's stricter constraint via the effective value computed below:
                // min(child, parent - 1).
            }
        } else {
            // Leaf certificate (last in chain)
            // pathLenConstraint must be undefined (already validated in verify_certificate_extensions)
            // digitalSignature bit must be set (already validated in verify_certificate_extensions)
            if max_path_len.is_some() {
                return Err("nitro: leaf certificate path must be undefined");
            }
        }

        // Update parent_max_path_len for the next iteration
        // For CA certs, use the effective maxPathLen (parent - 1 if parent was defined)
        if is_ca {
            parent_max_path_len = match (parent_max_path_len, max_path_len) {
                (Some(parent_max), Some(child_max)) => {
                    // Constrain to the more restrictive value: min(child, parent - 1)
                    Some(child_max.min(parent_max.saturating_sub(1)))
                }
                (Some(parent_max), None) => {
                    // Child has no constraint, but a parent limits it to parent - 1
                    Some(parent_max.saturating_sub(1))
                }
                (None, Some(child_max)) => Some(child_max),
                (None, None) => None, // Unlimited
            };
        }

        // Verify signature (except for root, which we trust)
        if i > 0 {
            verify_certificate(&chain[i], &chain[i - 1])?;
        }
    }

    Ok(())
}

fn verify_cosesign1(cosesign1: &CoseSign1, certificate: &Certificate) -> Result<(), &'static str> {
    let verifying_key = certificate
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or("nitro: could not get issuer public key")?;
    cosesign1.verify_signature(&[], |signature, signed_data| {
        let Ok(signature) = p384::ecdsa::Signature::from_bytes(signature.into()) else {
            return Err("nitro: invalid ECDSA signature");
        };
        let Ok(verify_key) = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key) else {
            return Err("nitro: invalid ECDSA public key");
        };
        if !verify_key.verify(signed_data, &signature).is_ok() {
            return Err("nitro: invalid ECDSA signature");
        }
        Ok(())
    })
}

pub fn parse_attestation_and_verify(
    slice: &[u8],
    current_timestamp: u64,
) -> Result<AttestationDoc, &'static str> {
    let Ok(sign1) = CoseSign1::from_slice(slice) else {
        return Err("nitro: not a CoseSign1");
    };
    let Some(sign_payload) = sign1.payload.as_ref() else {
        return Err("nitro: missing sign payload");
    };
    let Some(doc) = AttestationDoc::from_slice(sign_payload) else {
        return Err("nitro: malformed attestation document");
    };

    if let Err(err) = validate_attestation_document(&doc) {
        return Err(err);
    }

    let Ok(root_cert) = Certificate::from_pem(NITRO_ROOT_CA_BYTES) else {
        return Err("nitro: failed to parse root certificate");
    };
    verify_attestation_doc(&doc, &root_cert, current_timestamp)?;

    let Ok(cert) = Certificate::from_der(doc.certificate.as_slice()) else {
        return Err("nitro: failed to parse certificate");
    };
    verify_cosesign1(&sign1, &cert)?;

    Ok(doc)
}

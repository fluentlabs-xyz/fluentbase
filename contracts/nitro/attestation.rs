use alloc::{string::String, vec, vec::Vec};
use coset::{CborSerializable, CoseError, CoseSign1};
use der::{asn1::ObjectIdentifier, Decode, DecodePem, Encode};
use fluentbase_sdk::{crypto::crypto_keccak256, debug_log};
use p384::ecdsa::signature::Verifier;
use x509_cert::{certificate::Certificate, ext::pkix::BasicConstraints};

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
    pub fn from_slice(slice: &[u8]) -> Self {
        let value: ciborium::Value =
            ciborium::de::from_reader(slice).expect("attestation document should be cbor encoded");
        let mut doc = Self::default();
        for (key, value) in value
            .into_map()
            .expect("attestation document should be a cbor map")
            .into_iter()
        {
            match key
                .as_text()
                .expect("attestation document key should be a cbor text")
            {
                "module_id" => {
                    doc.module_id = value
                        .into_text()
                        .expect("attestation.module_id should be of text type");
                }
                "digest" => {
                    doc.digest = value
                        .into_text()
                        .expect("attestation.digest should be of text type");
                }
                "timestamp" => {
                    doc.timestamp = i128::from(
                        value
                            .into_integer()
                            .expect("attestation.timestamp should be an integer"),
                    ) as u64;
                }
                "pcrs" => {
                    doc.pcrs = value
                        .into_map()
                        .expect("attestation.pcrs should be a map")
                        .into_iter()
                        .map(|x| {
                            (
                                i128::from(
                                    x.0.into_integer()
                                        .expect("attestation.pcrs keys should be integers"),
                                ) as u64,
                                x.1.into_bytes()
                                    .expect("attestation.pcrs values should be bytes"),
                            )
                        })
                        .collect();
                }
                "certificate" => {
                    doc.certificate = value
                        .into_bytes()
                        .expect("attestation.certificate should be bytes");
                }
                "cabundle" => {
                    doc.cabundle = value
                        .into_array()
                        .expect("attestation.cabundle should be an array")
                        .into_iter()
                        .map(|x| {
                            x.into_bytes()
                                .expect("attestation.cabundle elements should be bytes")
                        })
                        .collect();
                }
                "public_key" => {
                    doc.public_key = match value {
                        ciborium::Value::Bytes(b) => Some(b),
                        ciborium::Value::Null => None,
                        _ => panic!("attestation.public_key should be bytes or null"),
                    };
                }
                "user_data" => {
                    doc.user_data = match value {
                        ciborium::Value::Bytes(b) => Some(b),
                        ciborium::Value::Null => None,
                        _ => panic!("attestation.user_data should be bytes or null"),
                    };
                }
                "nonce" => {
                    doc.nonce = match value {
                        ciborium::Value::Bytes(b) => Some(b),
                        ciborium::Value::Null => None,
                        _ => panic!("attestation.nonce should be bytes or null"),
                    };
                }
                _ => panic!("unexpected key encountered in attestation document"),
            }
        }
        doc
    }
}

/// Validates certificate extensions according to RFC 5280 and Nitro specification.
/// Both BasicConstraints and KeyUsage extensions must be present.
/// Returns the pathLenConstraint if present (None means unlimited).
/// Note: Critical flag is not enforced, aligning with reference:
#[cfg_attr(test, allow(dead_code))]
fn verify_certificate_extensions(cert: &Certificate, is_ca: bool) -> Option<u8> {
    let extensions = cert
        .tbs_certificate
        .extensions
        .as_ref()
        .expect("extensions must be present");

    let mut basic_constraints_found = false;
    let mut key_usage_found = false;
    let mut max_path_len = None;

    for ext in extensions {
        if ext.extn_id == BASIC_CONSTRAINTS_OID {
            // BasicConstraints extension found (critical flag is optional)
            basic_constraints_found = true;

            // Decode BasicConstraints from the extension value (which is an OctetString)
            let extn_bytes = ext.extn_value.as_bytes();
            let basic_constraints = BasicConstraints::from_der(extn_bytes)
                .expect("failed to decode basicConstraints extension");

            // Verify CA flag matches expected value
            assert!(
                basic_constraints.ca == is_ca,
                "basicConstraints CA flag mismatch"
            );

            // Extract pathLenConstraint if present
            if let Some(path_len) = basic_constraints.path_len_constraint {
                max_path_len = Some(path_len);
            }
        } else if ext.extn_id == KEY_USAGE_OID {
            // KeyUsage extension found (critical flag is optional)
            key_usage_found = true;

            // Decode KeyUsage as BitString from the extension value
            // The extension value is an OctetString containing a DER-encoded BitString
            let extn_bytes = ext.extn_value.as_bytes();

            // Parse the BitString manually (similar to Solidity code)
            // BitString DER format: [0x03] [length] [unused_bits: u8] [data...]
            // We expect tag 0x03 (BIT STRING), unused_bits should be 0x00 (full bytes)
            if extn_bytes.is_empty() {
                panic!("keyUsage extension value is empty");
            }

            // Verify it's a BIT STRING (tag 0x03)
            if extn_bytes[0] != 0x03 {
                panic!(
                    "keyUsage extension: expected BIT STRING (0x03), got {}",
                    extn_bytes[0]
                );
            }

            // Parse length (simple DER length encoding)
            let mut data_start = 2; // Skip tag (1 byte) and length (1 byte for short form)
            if extn_bytes.len() < data_start + 1 {
                panic!("keyUsage extension: invalid length");
            }

            // Check unused bits (used bits only affect the last byte)
            let _unused_bits = extn_bytes[data_start];

            // Skip unused_bits byte to get the actual data
            data_start += 1;
            if extn_bytes.len() <= data_start {
                panic!("keyUsage extension is empty");
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
                assert!(
                    (first_byte & 0x04) != 0,
                    "keyCertSign bit must be set for CA certificates"
                );
            } else {
                // For leaf certificates: digitalSignature (bit 0) must be set
                // Bit 0 is the leftmost bit, which corresponds to: 0x80 (binary: 10000000)
                assert!(
                    (first_byte & 0x80) != 0,
                    "digitalSignature bit must be set for leaf certificates"
                );
            }
        }
    }

    assert!(
        basic_constraints_found,
        "basicConstraints extension not found"
    );
    assert!(key_usage_found, "keyUsage extension not found");

    // For leaf certificates, pathLenConstraint must not be present
    if !is_ca {
        assert!(
            max_path_len.is_none(),
            "pathLenConstraint must be undefined for client cert"
        );
    }

    max_path_len
}

fn verify_certificate(subject: &Certificate, issuer: &Certificate) {
    let signed_data = subject.tbs_certificate.to_der().unwrap();
    let signature = subject
        .signature
        .as_bytes()
        .ok_or("Could not get cert signature")
        .unwrap();
    let verifying_key = issuer
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .unwrap();

    match subject.signature_algorithm.oid {
        ecdsa::ECDSA_SHA384_OID => {
            let signature = p384::ecdsa::DerSignature::try_from(signature).unwrap();
            let verify_key = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key).unwrap();

            verify_key.verify(&signed_data, &signature).unwrap();
            debug_log!("verified key");
        }
        _ => {
            panic!("Unsupported ECDSA algorithm");
        }
    };
}

/// Verifies that a certificate is valid at the given timestamp.
/// Checks that notBefore <= timestamp <= notAfter.
fn verify_certificate_validity(cert: &Certificate, current_timestamp: u64) {
    let validity = &cert.tbs_certificate.validity;

    // Convert certificate validity times to Unix timestamps
    // x509-cert uses Time which can be UTCTime or GeneralizedTime
    // Both represent time as seconds since Unix epoch
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

    assert!(
        not_before <= current_timestamp,
        "certificate not valid yet: notBefore={}, current={}",
        not_before,
        current_timestamp
    );

    assert!(
        not_after >= current_timestamp,
        "certificate not valid anymore: notAfter={}, current={}",
        not_after,
        current_timestamp
    );
}

/// Validates attestation document fields according to the specification.
/// This implements the validation requirements from validateAttestation in the Solidity reference.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn validate_attestation_document(doc: &AttestationDoc) {
    // require(ptrs.moduleID.length() > 0, "no module id");
    assert!(!doc.module_id.is_empty(), "no module id");

    // require(ptrs.timestamp > 0, "no timestamp");
    assert!(doc.timestamp > 0, "no timestamp");

    // require(ptrs.cabundle.length > 0, "no cabundle");
    assert!(!doc.cabundle.is_empty(), "no cabundle");

    // require(attestationTbs.keccak(ptrs.digest) == ATTESTATION_DIGEST, "invalid digest");
    // ATTESTATION_DIGEST is keccak256("SHA384")
    let digest_hash = crypto_keccak256(doc.digest.as_bytes());
    assert!(
        digest_hash.as_slice() == ATTESTATION_DIGEST,
        "invalid digest: expected keccak256('SHA384'), got keccak256('{}')",
        doc.digest
    );

    // require(1 <= ptrs.pcrs.length && ptrs.pcrs.length <= 32, "invalid pcrs");
    assert!(
        (1..=32).contains(&doc.pcrs.len()),
        "invalid pcrs: length must be between 1 and 32, got {}",
        doc.pcrs.len()
    );

    // require(ptrs.publicKey.isNull() || (1 <= ptrs.publicKey.length() && ptrs.publicKey.length() <= 1024), "invalid pub key");
    if let Some(ref public_key) = doc.public_key {
        assert!(
            (1..=1024).contains(&public_key.len()),
            "invalid pub key: length must be between 1 and 1024, got {}",
            public_key.len()
        );
    }

    // require(ptrs.userData.isNull() || (ptrs.userData.length() <= 512), "invalid user data");
    if let Some(ref user_data) = doc.user_data {
        assert!(
            user_data.len() <= 512,
            "invalid user data: length must be <= 512, got {}",
            user_data.len()
        );
    }

    // require(ptrs.nonce.isNull() || (ptrs.nonce.length() <= 512), "invalid nonce");
    if let Some(ref nonce) = doc.nonce {
        assert!(
            nonce.len() <= 512,
            "invalid nonce: length must be <= 512, got {}",
            nonce.len()
        );
    }

    // Validate each PCR length
    // require(ptrs.pcrs[i].length() == 32 || ptrs.pcrs[i].length() == 48 || ptrs.pcrs[i].length() == 64, "invalid pcr");
    for (pcr_index, (_, pcr_value)) in doc.pcrs.iter().enumerate() {
        assert!(
            pcr_value.len() == 32 || pcr_value.len() == 48 || pcr_value.len() == 64,
            "invalid pcr at index {}: length must be 32, 48, or 64 bytes, got {}",
            pcr_index,
            pcr_value.len()
        );
    }

    // Validate each cabundle certificate length
    // require(1 <= ptrs.cabundle[i].length() && ptrs.cabundle[i].length() <= 1024, "invalid cabundle cert");
    for (i, cert_bytes) in doc.cabundle.iter().enumerate() {
        assert!(
            (1..=1024).contains(&cert_bytes.len()),
            "invalid cabundle cert at index {}: length must be between 1 and 1024, got {}",
            i,
            cert_bytes.len()
        );
    }

    // Validate certificate length (from doc.certificate)
    // The certificate should also be validated, similar to cabundle certs
    assert!(
        (1..=1024).contains(&doc.certificate.len()),
        "invalid certificate: length must be between 1 and 1024, got {}",
        doc.certificate.len()
    );
}

fn verify_attestation_doc(
    doc: &AttestationDoc,
    root_certificate: &Certificate,
    current_timestamp: u64,
) {
    let mut chain = Vec::new();
    assert!(!doc.cabundle.is_empty());
    assert_eq!(doc.cabundle[0], root_certificate.to_der().unwrap());
    for cert in &doc.cabundle {
        chain.push(Certificate::from_der(cert).unwrap());
    }
    chain.push(Certificate::from_der(&doc.certificate).unwrap());

    let chain_size = chain.len();
    // Chain size excluding root and leaf = number of intermediate CA certificates
    // The root is at index 0, leaf is at index chain.len() - 1
    let ca_chain_length = chain_size - 2; // Excluding root (index 0) and leaf (last index)

    // Track parent maxPathLen for pathLenConstraint validation
    let mut parent_max_path_len: Option<u8> = None;

    // Verify extensions, validity, and signatures for each certificate in the chain
    for (i, cert) in chain.iter().enumerate() {
        let is_ca = i < chain.len() - 1; // All certs except the last are CA certs

        // Verify certificate validity period (Section 3.2.3.1)
        verify_certificate_validity(cert, current_timestamp);

        let max_path_len = verify_certificate_extensions(cert, is_ca);

        if i == 0 {
            // Root certificate (index 0) specific validation
            // pathLenConstraint must be >= chain size (number of CA certs in chain excluding root)
            // If pathLenConstraint is undefined (None), it's unlimited, which is valid
            if let Some(root_path_len) = max_path_len {
                assert!(
                    root_path_len as usize >= ca_chain_length,
                    "root certificate pathLenConstraint ({}) must be >= chain size ({})",
                    root_path_len,
                    ca_chain_length
                );
            }
            // Root certificate should have keyCertSign (already validated in verify_certificate_extensions)
        } else if is_ca {
            // Intermediate CA certificates (all except root and leaf)
            // Check that pathLenConstraint is not exceeded
            if let Some(parent_max) = parent_max_path_len {
                // Parent's pathLenConstraint limits how many more CA certs can follow
                // If parent_max is 0, no more certificates can follow (chain already too long)
                assert!(
                    parent_max > 0,
                    "pathLenConstraint exceeded: parent certificate allows no more certificates"
                );
                // Child's pathLenConstraint, if defined, must be less than parent's
                if let Some(child_max) = max_path_len {
                    assert!(
                        child_max < parent_max,
                        "pathLenConstraint exceeded: child maxPathLen ({}) must be less than parent's ({})",
                        child_max,
                        parent_max
                    );
                }
            }
        } else {
            // Leaf certificate (last in chain)
            // pathLenConstraint must be undefined (already validated in verify_certificate_extensions)
            // digitalSignature bit must be set (already validated in verify_certificate_extensions)
            assert!(
                max_path_len.is_none(),
                "leaf certificate pathLenConstraint must be undefined"
            );
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
                    // Child has no constraint, but parent limits it to parent - 1
                    Some(parent_max.saturating_sub(1))
                }
                (None, Some(child_max)) => Some(child_max),
                (None, None) => None, // Unlimited
            };
        }

        // Verify signature (except for root, which we trust)
        if i > 0 {
            verify_certificate(&chain[i], &chain[i - 1]);
        }
    }
}

fn verify_cosesign1(cosesign1: &CoseSign1, certificate: &Certificate) {
    let verifying_key = certificate
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .unwrap();
    cosesign1
        .verify_signature(&vec![], |signature, signed_data| {
            let signature = p384::ecdsa::Signature::from_bytes(signature.into()).unwrap();
            let verify_key = p384::ecdsa::VerifyingKey::from_sec1_bytes(verifying_key).unwrap();
            verify_key.verify(&signed_data, &signature).unwrap();
            Ok::<(), CoseError>(())
        })
        .unwrap();
}

pub fn parse_and_verify(slice: &[u8], current_timestamp: u64) -> AttestationDoc {
    debug_log!("parsing sign");
    let sign1 = coset::CoseSign1::from_slice(slice).unwrap();
    debug_log!("parsing doc");
    let doc = AttestationDoc::from_slice(sign1.payload.as_ref().unwrap());
    debug_log!("validating attestation document");
    validate_attestation_document(&doc);
    debug_log!("parsing CA certificate");
    let root_cert = Certificate::from_pem(NITRO_ROOT_CA_BYTES).unwrap();
    debug_log!("verifying CA certificate");
    verify_attestation_doc(&doc, &root_cert, current_timestamp);
    debug_log!("parsing certificate");
    let cert = Certificate::from_der(doc.certificate.as_slice()).unwrap();
    debug_log!("verifying certificate");
    verify_cosesign1(&sign1, &cert);
    debug_log!("all done");
    doc
}

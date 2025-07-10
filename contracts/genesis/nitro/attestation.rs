use alloc::{string::String, vec, vec::Vec};
use coset::{CborSerializable, CoseError, CoseSign1};
use der::{Decode, DecodePem, Encode};
use fluentbase_sdk::debug_log;
use p384::ecdsa::signature::Verifier;
use x509_cert::certificate::Certificate;

static NITRO_ROOT_CA_BYTES: &[u8] = include_bytes!("nitro.pem");

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

fn verify_attestation_doc(doc: &AttestationDoc, root_certificate: &Certificate) {
    let mut chain = Vec::new();
    assert!(!doc.cabundle.is_empty());
    assert_eq!(doc.cabundle[0], root_certificate.to_der().unwrap());
    for cert in &doc.cabundle {
        chain.push(Certificate::from_der(cert).unwrap());
    }
    chain.push(Certificate::from_der(&doc.certificate).unwrap());
    for i in 0..chain.len() - 1 {
        verify_certificate(&chain[i + 1], &chain[i]);
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

pub fn parse_and_verify(slice: &[u8]) -> AttestationDoc {
    debug_log!("parsing sign");
    let sign1 = coset::CoseSign1::from_slice(slice).unwrap();
    debug_log!("parsing doc");
    let doc = AttestationDoc::from_slice(sign1.payload.as_ref().unwrap());
    debug_log!("parsing CA certificate");
    let root_cert = Certificate::from_pem(NITRO_ROOT_CA_BYTES).unwrap();
    debug_log!("verifying CA certificate");
    verify_attestation_doc(&doc, &root_cert);
    debug_log!("parsing certificate");
    let cert = Certificate::from_der(doc.certificate.as_slice()).unwrap();
    debug_log!("verifying certificate");
    verify_cosesign1(&sign1, &cert);
    debug_log!("all done");
    doc
}

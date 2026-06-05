//! EIP-2335 BLS12-381 keystore conformance vectors.
//!
//! Vectors are the canonical Appendix A from
//! <https://eips.ethereum.org/EIPS/eip-2335> — both scrypt and PBKDF2-HMAC-SHA256
//! KDFs, sharing the same 32-byte secret and password. If these break after
//! an eth-keystore / crypto-primitive bump, regenerate AND audit
//! cross-language parity.

use fluentbase_bls::keystore::EthKeystoreV4;

const EIP2335_SCRYPT_VECTOR: &str = include_str!("vectors/eip2335_scrypt.json");
const EIP2335_PBKDF2_VECTOR: &str = include_str!("vectors/eip2335_pbkdf2.json");
const EXPECTED_SECRET_HEX: &str =
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
const PASSWORD: &str = "𝔱𝔢𝔰𝔱𝔭𝔞𝔰𝔰𝔴𝔬𝔯𝔡🔑";

#[test]
fn eip2335_scrypt_vector_decrypts_to_expected_secret() {
    let ks = EthKeystoreV4::from_json(EIP2335_SCRYPT_VECTOR).unwrap();
    let secret = ks.decrypt(PASSWORD.as_bytes()).unwrap();
    assert_eq!(hex::encode(secret.as_slice()), EXPECTED_SECRET_HEX);
}

#[test]
fn eip2335_pbkdf2_vector_decrypts_to_expected_secret() {
    let ks = EthKeystoreV4::from_json(EIP2335_PBKDF2_VECTOR).unwrap();
    let secret = ks.decrypt(PASSWORD.as_bytes()).unwrap();
    assert_eq!(hex::encode(secret.as_slice()), EXPECTED_SECRET_HEX);
}

#[test]
fn wrong_password_fails_checksum() {
    let ks = EthKeystoreV4::from_json(EIP2335_SCRYPT_VECTOR).unwrap();
    let err = ks.decrypt(b"wrong-password").unwrap_err();
    assert!(matches!(err, fluentbase_bls::Error::KeystoreChecksum));
}

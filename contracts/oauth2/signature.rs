extern crate alloc;

use crate::{errors::OAuth2Error, jwks::JWK};
use alloc::vec::Vec;
use fluentbase_sdk::Bytes;
use sha2::{Digest, Sha256};

/// Verify JWT signature
pub fn verify_signature(
    message: &[u8],
    signature: &[u8],
    jwk: &JWK,
    algorithm: &str,
    gas_limit: u64,
) -> Result<bool, OAuth2Error> {
    match algorithm {
        "RS256" => verify_rs256(message, signature, jwk, gas_limit),
        "ES256" => verify_es256(message, signature, jwk, gas_limit),
        "RS384" => verify_rs384(message, signature, jwk, gas_limit),
        _ => Err(OAuth2Error::UnsupportedAlgorithm),
    }
}

/// Verify RS256 signature (RSA + SHA256)
///
/// Uses modexp precompile for RSA signature verification via the formula:
/// signature^e mod n == PKCS1_v1_5_padding(SHA256(message))
fn verify_rs256(
    message: &[u8],
    signature: &[u8],
    jwk: &JWK,
    gas_limit: u64,
) -> Result<bool, OAuth2Error> {
    use crate::jwks::decode_rsa_key;

    // Decode RSA public key from JWK
    let (n_bytes, e_bytes) = decode_rsa_key(jwk)?;

    // Validate input lengths
    if signature.is_empty() || n_bytes.is_empty() || e_bytes.is_empty() {
        return Err(OAuth2Error::InvalidSignature);
    }

    // RSA signature should be same length as modulus
    if signature.len() != n_bytes.len() {
        return Err(OAuth2Error::InvalidSignature);
    }

    // Hash the message using SHA256
    let mut hasher = Sha256::new();
    hasher.update(message);
    let hash = hasher.finalize();

    // Apply PKCS#1 v1.5 padding to hash
    let padded_hash = pkcs1_v15_pad_sha256(&hash, n_bytes.len())?;

    // Verify using modexp: signature^e mod n should equal padded_hash
    let result = rsa_verify_with_modexp(signature, &e_bytes, &n_bytes, &padded_hash, gas_limit)?;

    Ok(result)
}

/// Apply PKCS#1 v1.5 padding for SHA256 hash
/// Format: 0x00 || 0x01 || PS || 0x00 || DigestInfo || hash
/// where DigestInfo = 0x3031300d060960864801650304020105000420 for SHA256
pub(crate) fn pkcs1_v15_pad_sha256(hash: &[u8], target_len: usize) -> Result<Vec<u8>, OAuth2Error> {
    // DigestInfo for SHA256
    const DIGEST_INFO: &[u8] = &[
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];

    let t_len = DIGEST_INFO.len() + hash.len();
    if target_len < t_len + 11 {
        return Err(OAuth2Error::InvalidSignature);
    }

    let ps_len = target_len - t_len - 3;
    let mut padded = Vec::with_capacity(target_len);

    // 0x00 || 0x01
    padded.push(0x00);
    padded.push(0x01);

    // Padding string (0xff bytes)
    for _ in 0..ps_len {
        padded.push(0xff);
    }

    // 0x00 separator
    padded.push(0x00);

    // DigestInfo
    padded.extend_from_slice(DIGEST_INFO);

    // Hash
    padded.extend_from_slice(hash);

    Ok(padded)
}

/// Verify RSA signature using modexp precompile
/// Computes signature^e mod n and compares with expected value
fn rsa_verify_with_modexp(
    signature: &[u8],
    exponent: &[u8],
    modulus: &[u8],
    expected: &[u8],
    gas_limit: u64,
) -> Result<bool, OAuth2Error> {
    use num_bigint::BigUint;

    // Prepare input for modexp: len(base) || len(exp) || len(mod) || base || exp || mod
    // Format: 32 bytes for each length, then the actual data
    let mut input = Vec::new();

    let sig_len = signature.len();
    let exp_len = exponent.len();
    let mod_len = modulus.len();

    // Encode lengths as 32-byte big-endian integers
    let mut sig_len_bytes = [0u8; 32];
    let sig_len_u256 = BigUint::from(sig_len);
    let sig_len_be = sig_len_u256.to_bytes_be();
    sig_len_bytes[32 - sig_len_be.len()..].copy_from_slice(&sig_len_be);
    input.extend_from_slice(&sig_len_bytes);

    let mut exp_len_bytes = [0u8; 32];
    let exp_len_u256 = BigUint::from(exp_len);
    let exp_len_be = exp_len_u256.to_bytes_be();
    exp_len_bytes[32 - exp_len_be.len()..].copy_from_slice(&exp_len_be);
    input.extend_from_slice(&exp_len_bytes);

    let mut mod_len_bytes = [0u8; 32];
    let mod_len_u256 = BigUint::from(mod_len);
    let mod_len_be = mod_len_u256.to_bytes_be();
    mod_len_bytes[32 - mod_len_be.len()..].copy_from_slice(&mod_len_be);
    input.extend_from_slice(&mod_len_bytes);

    // Append data fields
    input.extend_from_slice(signature);
    input.extend_from_slice(exponent);
    input.extend_from_slice(modulus);

    // Call modexp precompile
    let input_bytes = Bytes::copy_from_slice(&input);
    let result = revm_precompile::modexp::berlin_run(&input_bytes, gas_limit)
        .map_err(|_| OAuth2Error::InvalidSignature)?;

    // Compare result with expected padded hash
    // The result should be the same length as the modulus
    let result_bytes = result.bytes.as_ref();

    // Pad expected to modulus length if needed
    let mut expected_padded = expected.to_vec();
    while expected_padded.len() < modulus.len() {
        expected_padded.insert(0, 0);
    }

    // Compare the decrypted signature with the expected padded hash
    Ok(result_bytes == expected_padded.as_slice())
}

/// Verify ES256 signature (ECDSA P-256 + SHA256)
///
/// Uses P-256 elliptic curve signature verification
fn verify_es256(
    message: &[u8],
    signature: &[u8],
    jwk: &JWK,
    _gas_limit: u64,
) -> Result<bool, OAuth2Error> {
    use crate::jwks::decode_ec_key;

    // Decode EC public key from JWK
    let (x_bytes, y_bytes) = decode_ec_key(jwk)?;

    // Validate curve is P-256
    if let Some(crv) = &jwk.crv {
        if crv != "P-256" {
            return Err(OAuth2Error::UnsupportedAlgorithm);
        }
    }

    // ES256 signature is 64 bytes (r || s, each 32 bytes)
    if signature.len() != 64 {
        return Err(OAuth2Error::InvalidSignature);
    }

    // Hash the message using SHA256
    let mut hasher = Sha256::new();
    hasher.update(message);
    let hash = hasher.finalize();

    // Verify ECDSA signature
    // For production, this should use a proper ECDSA verification library
    // or call the secp256r1 precompile if available
    let is_valid = verify_ecdsa_p256(&hash, signature, &x_bytes, &y_bytes)?;

    Ok(is_valid)
}

/// Verify RS384 signature (RSA + SHA384)
///
/// Similar to RS256 but uses SHA384 hash
fn verify_rs384(
    message: &[u8],
    signature: &[u8],
    jwk: &JWK,
    gas_limit: u64,
) -> Result<bool, OAuth2Error> {
    use crate::jwks::decode_rsa_key;
    use sha2::Sha384;

    // Decode RSA public key from JWK
    let (n_bytes, e_bytes) = decode_rsa_key(jwk)?;

    // Validate input lengths
    if signature.is_empty() || n_bytes.is_empty() || e_bytes.is_empty() {
        return Err(OAuth2Error::InvalidSignature);
    }

    if signature.len() != n_bytes.len() {
        return Err(OAuth2Error::InvalidSignature);
    }

    // Hash the message using SHA384
    let mut hasher = Sha384::new();
    hasher.update(message);
    let hash = hasher.finalize();

    // Apply PKCS#1 v1.5 padding for SHA384
    let padded_hash = pkcs1_v15_pad_sha384(&hash, n_bytes.len())?;

    // Verify using modexp
    let result = rsa_verify_with_modexp(signature, &e_bytes, &n_bytes, &padded_hash, gas_limit)?;

    Ok(result)
}

/// Apply PKCS#1 v1.5 padding for SHA384 hash
fn pkcs1_v15_pad_sha384(hash: &[u8], target_len: usize) -> Result<Vec<u8>, OAuth2Error> {
    // DigestInfo for SHA384
    const DIGEST_INFO: &[u8] = &[
        0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02,
        0x05, 0x00, 0x04, 0x30,
    ];

    let t_len = DIGEST_INFO.len() + hash.len();
    if target_len < t_len + 11 {
        return Err(OAuth2Error::InvalidSignature);
    }

    let ps_len = target_len - t_len - 3;
    let mut padded = Vec::with_capacity(target_len);

    padded.push(0x00);
    padded.push(0x01);

    for _ in 0..ps_len {
        padded.push(0xff);
    }

    padded.push(0x00);
    padded.extend_from_slice(DIGEST_INFO);
    padded.extend_from_slice(hash);

    Ok(padded)
}

/// Verify ECDSA P-256 signature using secp256r1 precompile
///
/// Uses revm_precompile::secp256r1::p256_verify for verification
fn verify_ecdsa_p256(
    hash: &[u8],
    signature: &[u8],
    x: &[u8],
    y: &[u8],
) -> Result<bool, OAuth2Error> {
    // Validate input lengths
    if hash.len() != 32 {
        return Err(OAuth2Error::InvalidSignature);
    }
    if signature.len() != 64 {
        return Err(OAuth2Error::InvalidSignature);
    }
    if x.len() != 32 || y.len() != 32 {
        return Err(OAuth2Error::InvalidSignature);
    }

    // Build precompile input: hash || r || s || x || y
    // Total: 32 + 32 + 32 + 32 + 32 = 160 bytes
    let mut precompile_input = Vec::with_capacity(160);
    precompile_input.extend_from_slice(hash); // message hash (32 bytes)
    precompile_input.extend_from_slice(&signature[..32]); // r (32 bytes)
    precompile_input.extend_from_slice(&signature[32..]); // s (32 bytes)
    precompile_input.extend_from_slice(x); // public key x (32 bytes)
    precompile_input.extend_from_slice(y); // public key y (32 bytes)

    // Call secp256r1 precompile
    let result = revm_precompile::secp256r1::p256_verify(&precompile_input, u64::MAX)
        .map_err(|_| OAuth2Error::InvalidSignature)?;

    // Precompile returns a single byte: 0x01 if valid, empty if invalid
    Ok(!result.bytes.is_empty() && result.bytes[0] == 0x01)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwks::JWK;

    #[test]
    fn test_verify_signature_unsupported_alg() {
        let message = b"test message";
        let signature = vec![0u8; 256];
        let jwk = JWK {
            kty: "RSA".into(),
            alg: Some("RS512".into()),
            kid: Some("test".into()),
            key_use: Some("sig".into()),
            n: Some("test".into()),
            e: Some("AQAB".into()),
            crv: None,
            x: None,
            y: None,
        };

        let result = verify_signature(message, &signature, &jwk, "RS512", 100_000);
        assert!(matches!(result, Err(OAuth2Error::UnsupportedAlgorithm)));
    }

    #[test]
    fn test_pkcs1_v15_padding() {
        // Test PKCS#1 v1.5 padding for SHA256
        let hash = [0x12u8; 32]; // Dummy hash
        let padded = pkcs1_v15_pad_sha256(&hash, 256).unwrap();

        // Check structure
        assert_eq!(padded[0], 0x00);
        assert_eq!(padded[1], 0x01);
        assert_eq!(padded[padded.len() - 32..], hash); // Hash at the end

        // Check padding bytes are 0xff
        let ps_start = 2;
        let ps_end = padded.len() - 32 - 19 - 1; // Before 0x00 separator
        for i in ps_start..ps_end {
            assert_eq!(padded[i], 0xff, "Padding byte at {} should be 0xff", i);
        }
    }

    #[test]
    fn test_real_google_rsa_key_decode() {
        // Test decoding real Google RSA public key
        use crate::jwks::decode_rsa_key;
        use crate::jwks_data;

        let google_jwks = jwks_data::google_jwks();
        let key = google_jwks.keys.first().expect("Google should have keys");

        let result = decode_rsa_key(key);
        assert!(result.is_ok(), "Should decode Google's real RSA key");

        let (n, e) = result.unwrap();

        // Verify key properties
        assert!(
            n.len() >= 256,
            "Google uses 2048-bit RSA (256 bytes minimum)"
        );
        assert_eq!(e, vec![0x01, 0x00, 0x01], "Exponent should be 65537");

        println!("Real Google RSA key properties:");
        println!("  Modulus: {} bytes", n.len());
        println!("  Exponent: {:?} (65537)", e);
    }

    #[test]
    fn test_modexp_simple() {
        // Test modexp with simple known values: 3^2 mod 5 = 4
        use num_bigint::BigUint;

        let base = vec![3u8];
        let exp = vec![2u8];
        let modulus = vec![5u8];

        // Build modexp input
        let mut input = Vec::new();

        // Lengths as 32-byte big-endian
        let mut len_bytes = [0u8; 32];
        len_bytes[31] = 1; // Length = 1
        input.extend_from_slice(&len_bytes); // base length
        input.extend_from_slice(&len_bytes); // exp length
        input.extend_from_slice(&len_bytes); // mod length

        // Data
        input.extend_from_slice(&base);
        input.extend_from_slice(&exp);
        input.extend_from_slice(&modulus);

        // Call modexp
        let input_bytes = Bytes::copy_from_slice(&input);
        let result = revm_precompile::modexp::berlin_run(&input_bytes, 100_000);

        assert!(result.is_ok(), "Modexp should succeed");
        let output = result.unwrap();

        // Should be 4 (9 mod 5 = 4)
        assert_eq!(output.bytes.as_ref(), &[4u8]);
        println!("Modexp precompile: 3^2 mod 5 = 4");
    }

    #[test]
    fn test_rsa_signature_structure() {
        // Test RSA signature verification flow with real key structure
        use crate::jwks::decode_rsa_key;
        use crate::jwks_data;

        let google_jwks = jwks_data::google_jwks();
        let key = google_jwks.keys.first().unwrap();
        let (n, e) = decode_rsa_key(key).unwrap();

        // Create test message
        let message = b"test message for signing";
        let mut hasher = Sha256::new();
        hasher.update(message);
        let hash = hasher.finalize();

        // Create PKCS#1 v1.5 padded hash
        let padded = pkcs1_v15_pad_sha256(&hash, n.len()).unwrap();

        assert_eq!(padded.len(), n.len());
        println!("RSA verification structure correct");
        println!("  Hash: {} bytes", hash.len());
        println!("  Padded: {} bytes", padded.len());
        println!("  Modulus: {} bytes", n.len());
    }
}

use alloc::{format, string::String, vec::Vec};
use fluentbase_sdk::{codec::Codec, Bytes, ExitCode, U256};
use sha2::{Digest, Sha256};

/// WebAuthn authenticator data flag bits
pub const AUTH_DATA_FLAGS_UP: u8 = 0x01; // User Present bit
pub const AUTH_DATA_FLAGS_UV: u8 = 0x04; // User Verified bit
pub const AUTH_DATA_FLAGS_BE: u8 = 0x08; // Backup Eligibility bit
pub const AUTH_DATA_FLAGS_BS: u8 = 0x10; // Backup State bit

/// Minimum length of authenticator data (32 bytes RP ID hash + 1 byte flags + 4 bytes counter)
pub const AUTH_DATA_MIN_LENGTH: usize = 37;

/// Index of the flags byte in authenticator data
pub const AUTH_DATA_FLAGS_INDEX: usize = 32;

/// Structure for WebAuthn authentication verification
///
/// Based on the Solady implementation:
/// https://github.com/Vectorized/solady/blob/824ca8c2b2cf86ed017b314496cd514550f7e678/src/utils/WebAuthn.sol
#[derive(Codec, Debug, Clone)]
pub struct WebAuthnAuth {
    /// The WebAuthn authenticator data.
    /// Contains RP ID hash, flags, counter, and optionally AAGUID, credential ID, and credential
    /// public key. See: https://www.w3.org/TR/webauthn-2/#dom-authenticatorassertionresponse-authenticatordata
    pub authenticator_data: Bytes,

    /// The WebAuthn client data JSON.
    /// Contains type, challenge, origin, and other client data.
    /// See: https://www.w3.org/TR/webauthn-2/#dom-authenticatorresponse-clientdatajson
    pub client_data_json: Bytes,

    /// Start index of "challenge":"..." in `client_data_json`.
    /// Used to verify that the client data contains the correct challenge.
    pub challenge_index: U256,

    /// Start index of "type":"..." in `client_data_json`.
    /// Used to verify that the client data has the correct type (webauthn.get).
    pub type_index: U256,

    /// Signature components (r, s) of the WebAuthn authentication assertion.
    pub r: U256,
    pub s: U256,
}

/// Verifies a WebAuthn Authentication Assertion
///
/// Implements selective verification following W3C WebAuthn spec:
/// https://www.w3.org/TR/webauthn-2/#sctn-verifying-assertion
///
/// This implementation only verifies elements critical for blockchain use:
/// - User presence and verification flags
/// - Client data type and challenge
/// - Signature validity
///
/// Deliberately omits verifying:
/// - Origin and RP ID validation (delegated to authenticator)
/// - Credential backup state
/// - Extension outputs
/// - Signature counter
/// - Attestation objects
pub fn verify_webauthn(
    challenge: &Bytes,
    require_user_verification: bool,
    auth: &WebAuthnAuth,
    x: U256,
    y: U256,
    gas_limit: u64,
) -> Result<bool, ExitCode> {
    // Step 1: Verify client data JSON type and challenge existence
    if !verify_client_data_json(
        &auth.client_data_json,
        challenge,
        auth.type_index,
        auth.challenge_index,
    ) {
        return Ok(false);
    }

    // Step 2: Verify authenticator data flags
    if !verify_authenticator_flags(&auth.authenticator_data, require_user_verification) {
        return Ok(false);
    }

    // Step 3: Compute message hash
    let message_hash = compute_message_hash(&auth.authenticator_data, &auth.client_data_json[..]);

    // Step 4: Verify signature
    verify_signature(message_hash, auth.r, auth.s, x, y, gas_limit)
}

/// Verifies the client data JSON type and challenge
fn verify_client_data_json(
    client_data_json: &Bytes,
    challenge: &Bytes,
    type_index: U256,
    challenge_index: U256,
) -> bool {
    let type_idx = match u32::try_from(type_index) {
        Ok(idx) => idx as usize,
        Err(_) => return false,
    };

    let challenge_idx = match u32::try_from(challenge_index) {
        Ok(idx) => idx as usize,
        Err(_) => return false,
    };

    // Check if indices are within bounds
    if type_idx >= client_data_json.len() || challenge_idx >= client_data_json.len() {
        return false;
    }

    // Verify type is "webauthn.get"
    let type_str = b"\"type\":\"webauthn.get\"";
    if !contains_at(type_str, client_data_json, type_idx) {
        return false;
    }

    // Encode challenge in base64url format
    let encoded_challenge = base64url_encode(challenge);
    let challenge_str = format!("\"challenge\":\"{encoded_challenge}\"");

    // Verify challenge
    if !contains_at(challenge_str.as_bytes(), client_data_json, challenge_idx) {
        return false;
    }
    // Verify that the challenge is followed by a closing quote
    let expected_quote_pos = challenge_idx + challenge_str.len() - 1;
    if expected_quote_pos >= client_data_json.len() || client_data_json[expected_quote_pos] != b'"'
    {
        return false;
    }

    true
}

/// Verifies the authenticator data flags
fn verify_authenticator_flags(authenticator_data: &Bytes, require_user_verification: bool) -> bool {
    // Check minimum length (at least RP ID hash + flags + counter)
    if authenticator_data.len() < AUTH_DATA_MIN_LENGTH {
        return false;
    }

    // Get flags byte (located after the 32-byte RP ID hash)
    let flags = authenticator_data[AUTH_DATA_FLAGS_INDEX];

    // Check User Present flag (bit 0)
    if flags & AUTH_DATA_FLAGS_UP == 0 {
        return false;
    }

    // Check User Verified flag (bit 2) if required
    if require_user_verification && (flags & AUTH_DATA_FLAGS_UV == 0) {
        return false;
    }

    // Check Backup State bit and Backup Eligibility bit
    // Backup State bit must not be set if Backup Eligibility bit is not set
    if flags & AUTH_DATA_FLAGS_BE == 0 && flags & AUTH_DATA_FLAGS_BS != 0 {
        return false;
    }

    true
}

/// Computes the message hash: sha256(authenticator_data || sha256(client_data_json))
/// sign data; verify data hash and signature
fn compute_message_hash(authenticator_data: &[u8], client_data_json: &[u8]) -> [u8; 32] {
    // Compute SHA-256 hash of client_data_json
    let client_data_hash = sha256_hash(client_data_json);

    // Concatenate authenticator_data and client_data_hash
    let mut combined = Vec::with_capacity(authenticator_data.len() + client_data_hash.len());
    combined.extend_from_slice(authenticator_data);
    combined.extend_from_slice(&client_data_hash);

    // Compute SHA-256 hash of the combined data
    sha256_hash(&combined)
}

/// Verifies the signature using the secp256r1 precompile
fn verify_signature(
    message_hash: [u8; 32],
    r: U256,
    s: U256,
    x: U256,
    y: U256,
    gas_limit: u64,
) -> Result<bool, ExitCode> {
    // Prepare input for the secp256r1 precompile
    let mut input = Vec::with_capacity(160);
    input.extend_from_slice(&message_hash);
    input.extend_from_slice(&r.to_be_bytes::<32>());
    input.extend_from_slice(&s.to_be_bytes::<32>());
    input.extend_from_slice(&x.to_be_bytes::<32>());
    input.extend_from_slice(&y.to_be_bytes::<32>());

    let input_bytes = Bytes::copy_from_slice(&input);

    // Call the secp256r1 precompile
    let result = revm_precompile::secp256r1::p256_verify(&input_bytes, gas_limit)
        .map_err(|_| ExitCode::PrecompileError)?;

    // Check the result: if the last byte is 1, the signature is valid
    Ok(!result.bytes.is_empty() && result.bytes[result.bytes.len() - 1] == 1)
}

/// Computes the SHA-256 hash
fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Checks if a substring exists at a specific position in a string
fn contains_at(substr: &[u8], full_str: &[u8], location: usize) -> bool {
    let substr_len = substr.len();

    if location + substr_len > full_str.len() {
        return false;
    }

    let slice = &full_str[location..location + substr_len];
    slice == substr
}

/// Base64URL encode function
///
/// This function encodes binary data as base64url (URL-safe base64 without padding)
/// https://base64.guru/standards/base64url
pub fn base64url_encode(input: &[u8]) -> String {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::URL_SAFE_NO_PAD.encode(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::{
        ecdsa::{signature::SignerMut, SigningKey, VerifyingKey},
        elliptic_curve::rand_core::OsRng,
    };

    fn create_valid_client_data_json_test_data() -> (Bytes, Bytes, U256, U256) {
        // Create challenge from the Solidity contract test data
        let challenge_bytes =
            hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .expect("Failed to decode challenge hex");
        let challenge = Bytes::copy_from_slice(&challenge_bytes);

        // Encode challenge in base64url
        let encoded_challenge = base64url_encode(&challenge);

        // Create client_data_json (Safari format)
        let client_data_json_str = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"http://localhost:3005\"}}",
            encoded_challenge
        );

        let client_data_json = Bytes::copy_from_slice(client_data_json_str.as_bytes());

        // Indices from the Solidity contract test data
        let type_index = U256::from(1);
        let challenge_index = U256::from(23);

        (client_data_json, challenge, type_index, challenge_index)
    }

    fn create_valid_challenge() -> Bytes {
        Bytes::copy_from_slice(
            &hex::decode("f631058a3ba1116acce12396fad0a125b5041c43f8e15723709f81aa8d5f4ccf")
                .unwrap(),
        )
    }

    fn generate_key_pair() -> (SigningKey, VerifyingKey) {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        (signing_key, verifying_key)
    }

    fn extract_public_key_coordinates(verifying_key: &VerifyingKey) -> (Vec<u8>, Vec<u8>) {
        let encoded_point = verifying_key.to_encoded_point(false);
        let uncompressed_bytes = encoded_point.as_bytes();

        // Verify format (should start with 0x04 followed by 64 bytes)
        assert_eq!(
            uncompressed_bytes[0], 0x04,
            "Expected uncompressed format marker"
        );
        assert_eq!(
            uncompressed_bytes.len(),
            65,
            "Expected 65 bytes (1 + 32 + 32)"
        );

        // Extract the x and y coordinates (32 bytes each)
        let x = uncompressed_bytes[1..33].to_vec();
        let y = uncompressed_bytes[33..65].to_vec();

        (x, y)
    }

    /// Extracts r and s values from a signature
    fn extract_signature_components(signature: &p256::ecdsa::Signature) -> (Vec<u8>, Vec<u8>) {
        let signature_bytes = signature.to_bytes();

        // Check signature format - should be exactly 64 bytes (32 for r, 32 for s)
        assert_eq!(signature_bytes.len(), 64, "Expected 64-byte signature");

        let r = signature_bytes[0..32].to_vec();
        let s = signature_bytes[32..64].to_vec();

        (r, s)
    }

    /// Creates a valid WebAuthn authentication object
    fn create_valid_webauthn(challenge: &Bytes) -> (WebAuthnAuth, U256, U256) {
        // Create signature values (r and s)
        let (mut signing_key, verifying_key) = generate_key_pair();
        let (x, y) = extract_public_key_coordinates(&verifying_key);

        // SAFARI format
        let authenticator_data = Bytes::copy_from_slice(
            &hex::decode(
                "49960de5880e8c687434170f6476605b8fe4aeb9a28632c7995cf3ba831d97630500000101",
            )
            .expect("Failed to decode authenticator data"),
        );

        let client_data_json_str = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"http://localhost:3005\"}}",
            base64url_encode(&challenge)
        );

        let client_data_json = Bytes::copy_from_slice(client_data_json_str.as_bytes());

        // Compute SHA-256 hash of client_data_json
        let client_data_hash = sha256_hash(&client_data_json[..]);

        //  we sign msg = (authenticator_data || sha256(client_data_json))
        // NOTE: to verify the signature you need to use hash of the msg
        let mut msg_to_sign = Vec::with_capacity(authenticator_data.len() + client_data_hash.len());
        msg_to_sign.extend_from_slice(&authenticator_data);
        msg_to_sign.extend_from_slice(&client_data_hash);

        // Sign the message
        let signature = &signing_key.sign(&msg_to_sign);

        let (r, s) = extract_signature_components(&signature);

        (
            WebAuthnAuth {
                authenticator_data,
                client_data_json,
                challenge_index: U256::from(23),
                type_index: U256::from(1),
                r: U256::from_be_slice(&r),
                s: U256::from_be_slice(&s),
            },
            U256::from_be_slice(&x),
            U256::from_be_slice(&y),
        )
    }

    #[test]
    fn test_verify_client_data_json_valid() {
        // Get valid test data
        let (client_data_json, challenge, type_index, challenge_index) =
            create_valid_client_data_json_test_data();

        // Verify that the function returns true for valid data
        let result =
            verify_client_data_json(&client_data_json, &challenge, type_index, challenge_index);

        assert!(
            result,
            "verify_client_data_json should return true for valid data"
        );
    }

    #[test]
    fn test_verify_client_data_json_invalid() {
        // Get valid test data
        let (client_data_json, _, type_index, challenge_index) =
            create_valid_client_data_json_test_data();

        // Create an invalid challenge
        let wrong_challenge = Bytes::copy_from_slice(b"wrong challenge");

        // Verify that the function returns false for invalid data
        let result = verify_client_data_json(
            &client_data_json,
            &wrong_challenge,
            type_index,
            challenge_index,
        );

        assert!(
            !result,
            "verify_client_data_json should return false for invalid data"
        );
    }

    #[test]
    fn test_verify_authenticator_flags() {
        fn create_test_authenticator_data(flags: u8) -> Bytes {
            // 32 bytes RP ID hash + 1 byte flags + 4 bytes counter
            let mut auth_data = Vec::with_capacity(AUTH_DATA_MIN_LENGTH);

            auth_data.extend_from_slice(&[
                0x49, 0x96, 0x0d, 0xe5, 0x88, 0x0e, 0x8c, 0x68, 0x74, 0x34, 0x17, 0x0f, 0x64, 0x76,
                0x60, 0x5b, 0x8f, 0xe4, 0xae, 0xb9, 0xa2, 0x86, 0x32, 0xc7, 0x99, 0x5c, 0xf3, 0xba,
                0x83, 0x1d, 0x97, 0x63,
            ]);

            auth_data.push(flags);

            auth_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);

            Bytes::copy_from_slice(&auth_data)
        }

        // User Present bit set, User Verified bit set
        let auth_data_up_uv =
            create_test_authenticator_data(AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_UV);
        assert!(
            verify_authenticator_flags(&auth_data_up_uv, true),
            "Should pass with UP and UV flags set when UV is required"
        );

        // User Present bit set, User Verified bit not set, but not required
        let auth_data_up = create_test_authenticator_data(AUTH_DATA_FLAGS_UP);
        assert!(
            verify_authenticator_flags(&auth_data_up, false),
            "Should pass with only UP flag set when UV is not required"
        );

        // User Present bit set, User Verified bit not set, but required
        assert!(
            !verify_authenticator_flags(&auth_data_up, true),
            "Should fail with only UP flag set when UV is required"
        );

        // User Present bit not set
        let auth_data_none = create_test_authenticator_data(0);
        assert!(
            !verify_authenticator_flags(&auth_data_none, false),
            "Should fail with no flags set"
        );

        // Backup State bit set without Backup Eligibility bit
        let auth_data_bs = create_test_authenticator_data(AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_BS);
        assert!(
            !verify_authenticator_flags(&auth_data_bs, false),
            "Should fail with BS flag set but BE flag not set"
        );

        // Both Backup State and Backup Eligibility bits set
        let auth_data_bs_be = create_test_authenticator_data(
            AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_BS | AUTH_DATA_FLAGS_BE,
        );
        assert!(
            verify_authenticator_flags(&auth_data_bs_be, false),
            "Should pass with both BS and BE flags set"
        );
    }

    #[test]
    fn test_valid_signature() {
        let challenge = create_valid_challenge();
        let (auth, x, y) = create_valid_webauthn(&challenge);

        let result = verify_webauthn(&challenge, true, &auth, x, y, 100000);

        println!("verify_webauthn result: {:?}", result);
        assert!(
            result.is_ok(),
            "verify_webauthn should return Ok for valid signature"
        );
        assert!(
            result.unwrap(),
            "verify_webauthn should return true for valid signature"
        );

        // // UNCOMMENT THIS TO CREATE THE ABI ENCODED INPUT
        //
        // let mut buf = BytesMut::new();
        // let invalid_auth = WebAuthnAuth {
        //     authenticator_data: auth.authenticator_data.clone(),
        //     client_data_json: auth.client_data_json.clone(),
        //     challenge_index: auth.challenge_index,
        //     type_index: auth.type_index,
        //     r: U256::from(0), // !invalid signature
        //     s: U256::from(0), // !invalid signature
        // };
        // SolidityABI::<(Bytes, bool, WebAuthnAuth, U256, U256)>::encode(
        //     &(challenge.clone(), true, invalid_auth, x.clone(), y.clone()),
        //     &mut buf,
        //     0,
        // )
        // .expect("Failed to encode input");
        //
        // let params = buf.freeze();
        // println!("Params: {:?}", hex::encode(&params));
    }

    #[test]
    fn test_invalid_signature() {
        let challenge = create_valid_challenge();
        let (auth, x, y) = create_valid_webauthn(&challenge);

        // Modify the signature to make it invalid
        let mut auth_invalid = auth.clone();
        auth_invalid.r = U256::from(0);
        auth_invalid.s = U256::from(0);

        let result = verify_webauthn(&challenge, true, &auth_invalid, x, y, 100000);

        println!("verify_webauthn result: {:?}", result);
        assert!(
            result.is_ok(),
            "verify_webauthn should return Ok for invalid signature"
        );
        assert!(
            !result.unwrap(),
            "verify_webauthn should return false for invalid signature"
        );
    }
}

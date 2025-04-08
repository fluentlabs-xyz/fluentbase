extern crate alloc;

use crate::utils::{base64url_encode, compare_bytes, contains_at};
use alloc::vec::Vec;
use p256::{
    ecdsa::{signature::Verifier, Signature, VerifyingKey},
    EncodedPoint,
};
use sha2::{Digest, Sha256};

// WebAuthn authenticator data flag bits
pub const AUTH_DATA_FLAGS_UP: u8 = 0x01; // User Present bit
pub const AUTH_DATA_FLAGS_UV: u8 = 0x04; // User Verified bit
pub const AUTH_DATA_FLAGS_BE: u8 = 0x08; // Backup Eligibility bit
pub const AUTH_DATA_FLAGS_BS: u8 = 0x10; // Backup State bit

// P256 curve order n/2 for malleability check
pub const P256_N_DIV_2: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xDE, 0x73, 0x7D, 0x56, 0xD3, 0x8B, 0xCF, 0x42, 0x79, 0xDC, 0xE5, 0x61, 0x7E, 0x31, 0x92, 0xA8,
];

/// Create ECDSA signature from r and s components
pub fn create_signature(r: &[u8; 32], s: &[u8; 32]) -> Result<Signature, &'static str> {
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    // Convert bytes directly to signature
    Signature::from_bytes(&sig_bytes.into()).map_err(|_| "Invalid signature components")
}

/// Create verifying key from x and y coordinates
pub fn create_verifying_key(x: &[u8; 32], y: &[u8; 32]) -> Result<VerifyingKey, &'static str> {
    let mut point_bytes = [0u8; 65];
    point_bytes[0] = 0x04; // not compressed
    point_bytes[1..33].copy_from_slice(x);
    point_bytes[33..65].copy_from_slice(y);
    let encoded_point =
        EncodedPoint::from_bytes(point_bytes).map_err(|_| "Invalid point coordinates")?;
    VerifyingKey::from_encoded_point(&encoded_point).map_err(|_| "Invalid verifying key")
}

/// Compute SHA-256 hash
pub fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Validate authenticator data flags
pub fn check_auth_flags(flags: u8, require_user_verification: bool) -> bool {
    // User Present bit must be set
    if flags & AUTH_DATA_FLAGS_UP == 0 {
        return false;
    }

    // User Verification bit must be set if required
    if require_user_verification && (flags & AUTH_DATA_FLAGS_UV == 0) {
        return false;
    }

    // Backup State bit must not be set if Backup Eligibility bit is not set
    if flags & AUTH_DATA_FLAGS_BE == 0 && flags & AUTH_DATA_FLAGS_BS != 0 {
        return false;
    }

    true
}

/// Verify P256 signature
/// Selector - 0xc358910e
/// function verifyP256Signature(
///         bytes32 messageHash,
///         bytes32 r,
///         bytes32 s,
///         bytes32 x,
///         bytes32 y,
///         bool malleabilityCheck
///     )
pub fn verify_p256_signature(
    message_hash: &[u8; 32],
    r: &[u8; 32],
    s: &[u8; 32],
    x: &[u8; 32],
    y: &[u8; 32],
    malleability_check: bool,
) -> bool {
    // if s > n/2,
    if malleability_check && compare_bytes(s, &P256_N_DIV_2) > 0 {
        return false;
    };
    let signature = match create_signature(r, s) {
        Ok(sig) => sig,
        Err(_) => return false,
    };
    let verifying_key = match create_verifying_key(x, y) {
        Ok(key) => key,
        Err(_) => return false,
    };

    verifying_key
        .verify(message_hash.as_ref(), &signature)
        .is_ok()
}

/// Verify WebAuthn assertion
/// Selector - 0x01352e1d
/// function verifyWebauthn(
///         bytes calldata challenge,
///         bytes calldata authenticatorData,
///         bool requireUserVerification,
///         bytes calldata clientDataJSON,
///         uint32 challengeLocation,
///         uint32 responseTypeLocation,
///         bytes32 r,
///         bytes32 s,
///         bytes32 x,
///         bytes32 y
///     )
pub fn verify_webauthn(params: &crate::params::WebAuthnParams) -> bool {
    // Special case: If responseTypeLocation is u32::MAX, skip WebAuthn checks
    // and only verify the signature (dummy mode)
    let dummy_mode = params.response_type_location == u32::MAX;

    // Prepare message for signature verification (same in all cases)
    let client_data_json_hash = sha256_hash(params.client_data_json);
    let mut message_data = Vec::with_capacity(params.authenticator_data.len() + 32);
    message_data.extend_from_slice(params.authenticator_data);
    message_data.extend_from_slice(&client_data_json_hash);
    let message_hash = sha256_hash(&message_data);

    // In dummy mode, only verify the signature
    if dummy_mode {
        return verify_p256_signature(
            &message_hash,
            &params.r,
            &params.s,
            &params.x,
            &params.y,
            false,
        );
    }

    // Regular WebAuthn checks

    // 1. Check authenticator data length and flags
    if params.authenticator_data.len() < 37 {
        return false;
    }

    if !check_auth_flags(
        params.authenticator_data[32],
        params.require_user_verification,
    ) {
        return false;
    }

    // 2. Check that response is for an authentication assertion
    let response_type = b"\"type\":\"webauthn.get\"";
    let response_type_location_usize = params.response_type_location as usize;

    if !contains_at(
        response_type,
        params.client_data_json,
        response_type_location_usize,
    ) {
        return false;
    }

    // 3. Check that challenge is in the clientDataJSON
    let challenge_b64url = base64url_encode(params.challenge);
    let challenge_property = alloc::format!("\"challenge\":\"{}\"", challenge_b64url);
    let challenge_location_usize = params.challenge_location as usize;

    if !contains_at(
        challenge_property.as_bytes(),
        params.client_data_json,
        challenge_location_usize,
    ) {
        return false;
    }

    // 4. Finally, verify the signature
    verify_p256_signature(
        &message_hash,
        &params.r,
        &params.s,
        &params.x,
        &params.y,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::WebAuthnParams;
    use p256::{
        ecdsa::{signature::Signer, Signature, SigningKey},
        elliptic_curve::rand_core::OsRng,
    };

    // Test fixture structure for WebAuthn tests
    struct WebAuthnTestFixture {
        challenge: Vec<u8>,
        auth_data: Vec<u8>,
        client_data_json_bytes: Vec<u8>,
        challenge_location: u32,
        type_location: u32,
        r: [u8; 32],
        s: [u8; 32],
        x: [u8; 32],
        y: [u8; 32],
        s_is_high: bool,
    }

    impl WebAuthnTestFixture {
        fn create_params(&self) -> WebAuthnParams {
            WebAuthnParams {
                challenge: &self.challenge,
                authenticator_data: &self.auth_data,
                require_user_verification: true,
                client_data_json: &self.client_data_json_bytes,
                challenge_location: self.challenge_location,
                response_type_location: self.type_location,
                r: self.r,
                s: self.s,
                x: self.x,
                y: self.y,
            }
        }
    }

    // Function to create a standard test fixture
    fn create_test_fixture() -> WebAuthnTestFixture {
        // Create a consistent challenge
        let challenge = b"Test WebAuthn Challenge".to_vec();

        // Create authenticator data with UP and UV flags
        let mut auth_data = Vec::with_capacity(37);
        // RP ID hash (32 bytes)
        auth_data.extend_from_slice(&[
            0x49, 0x96, 0x0d, 0xe5, 0x88, 0x0e, 0x8c, 0x68, 0x74, 0x34, 0x17, 0x0f, 0x64, 0x76,
            0x60, 0x5b, 0x8f, 0xe4, 0xae, 0xb9, 0xa2, 0x86, 0x32, 0xc7, 0x99, 0x5c, 0xf3, 0xba,
            0x83, 0x1d, 0x97, 0x63,
        ]);
        // Set flags - User Present and User Verified
        auth_data.push(AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_UV);
        // Add counter
        auth_data.extend_from_slice(&[0x00, 0x00, 0x01, 0x01]);

        // Create client data JSON
        let challenge_b64 = base64url_encode(&challenge);
        let client_data_json = format!(
            "{{\"type\":\"webauthn.get\",\"challenge\":\"{}\",\"origin\":\"http://localhost\"}}",
            challenge_b64
        );
        let client_data_json_bytes = client_data_json.as_bytes().to_vec();

        // Find exact locations
        let challenge_location = client_data_json.find("\"challenge\":").unwrap() as u32;
        let type_location = client_data_json.find("\"type\":").unwrap() as u32;

        // Create a key pair that will produce a valid signature
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Create the message to sign
        let client_data_json_hash = sha256_hash(&client_data_json_bytes);
        let mut message_data = Vec::with_capacity(auth_data.len() + 32);
        message_data.extend_from_slice(&auth_data);
        message_data.extend_from_slice(&client_data_json_hash);
        let message_hash = sha256_hash(&message_data);

        // Sign the message
        let signature: Signature = signing_key.sign(&message_hash);

        // Extract signature components
        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // Check if s > n/2 (malleable signature)
        let s_is_high = compare_bytes(&s, &P256_N_DIV_2) > 0;

        // Extract key components
        let encoded_point = verifying_key.to_encoded_point(false);
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(encoded_point.x().unwrap());
        y.copy_from_slice(encoded_point.y().unwrap());

        WebAuthnTestFixture {
            challenge,
            auth_data,
            client_data_json_bytes,
            challenge_location,
            type_location,
            r,
            s,
            x,
            y,
            s_is_high,
        }
    }

    #[test]
    fn test_verify_webauthn_valid() {
        let fixture = create_test_fixture();

        // Skip test if s > n/2
        if fixture.s_is_high {
            println!("Skipping test_verify_webauthn_valid due to high s value");
            return;
        }

        let params = fixture.create_params();
        assert!(
            verify_webauthn(&params),
            "Valid WebAuthn assertion should verify successfully"
        );
    }

    #[test]
    fn test_verify_webauthn_short_auth_data() {
        let fixture = create_test_fixture();

        let short_auth_data = fixture.auth_data[..32].to_vec();

        let params = WebAuthnParams {
            challenge: &fixture.challenge,
            authenticator_data: &short_auth_data,
            require_user_verification: true,
            client_data_json: &fixture.client_data_json_bytes,
            challenge_location: fixture.challenge_location,
            response_type_location: fixture.type_location,
            r: fixture.r,
            s: fixture.s,
            x: fixture.x,
            y: fixture.y,
        };

        assert!(
            !verify_webauthn(&params),
            "Short authenticator data should fail"
        );
    }

    #[test]
    fn test_verify_webauthn_missing_uv_flag() {
        let fixture = create_test_fixture();

        let mut auth_data_no_uv = fixture.auth_data.clone();
        auth_data_no_uv[32] = AUTH_DATA_FLAGS_UP;

        let params = WebAuthnParams {
            challenge: &fixture.challenge,
            authenticator_data: &auth_data_no_uv,
            require_user_verification: true,
            client_data_json: &fixture.client_data_json_bytes,
            challenge_location: fixture.challenge_location,
            response_type_location: fixture.type_location,
            r: fixture.r,
            s: fixture.s,
            x: fixture.x,
            y: fixture.y,
        };

        assert!(
            !verify_webauthn(&params),
            "Missing required UV flag should fail"
        );
    }

    #[test]
    fn test_verify_webauthn_wrong_challenge() {
        let fixture = create_test_fixture();

        let wrong_challenge = b"Wrong Challenge".to_vec();

        let params = WebAuthnParams {
            challenge: &wrong_challenge,
            authenticator_data: &fixture.auth_data,
            require_user_verification: true,
            client_data_json: &fixture.client_data_json_bytes,
            challenge_location: fixture.challenge_location,
            response_type_location: fixture.type_location,
            r: fixture.r,
            s: fixture.s,
            x: fixture.x,
            y: fixture.y,
        };

        assert!(!verify_webauthn(&params), "Wrong challenge should fail");
    }

    #[test]
    fn test_verify_p256_signature() {
        // Create a new signing key directly in the test
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        // Create a test message and hash it
        let message = b"Test message";
        let message_hash = sha256_hash(message);

        // Sign the hash using the p256 library directly - fix: sign using the correct format
        let signature: Signature = signing_key.sign(message_hash.as_ref());

        // Extract signature components
        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // Extract key components
        let encoded_point = verifying_key.to_encoded_point(false);
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(encoded_point.x().unwrap());
        y.copy_from_slice(encoded_point.y().unwrap());

        // Use our function to verify the signature with the extracted components
        let valid = verify_p256_signature(&message_hash, &r, &s, &x, &y, false);
        assert!(valid, "Valid signature should be verified");

        // Test invalid cases
        let wrong_hash = sha256_hash(b"Wrong message");
        assert!(!verify_p256_signature(&wrong_hash, &r, &s, &x, &y, true));

        let mut tampered_r = r;
        tampered_r[0] ^= 0xFF;
        assert!(!verify_p256_signature(
            &message_hash,
            &tampered_r,
            &s,
            &x,
            &y,
            true
        ));

        // For the malleability check, create a high s value but don't try to construct
        // a valid signature with it, just test the check itself
        let high_s = [0xFF; 32]; // s > n/2
        assert!(!verify_p256_signature(
            &message_hash,
            &r,
            &high_s,
            &x,
            &y,
            true
        ));
    }

    #[test]
    fn test_check_auth_flags() {
        // Valid cases
        assert!(check_auth_flags(AUTH_DATA_FLAGS_UP, false));
        assert!(check_auth_flags(
            AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_UV,
            true
        ));
        assert!(check_auth_flags(
            AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_BE | AUTH_DATA_FLAGS_BS,
            false
        ));

        // Invalid cases
        assert!(!check_auth_flags(0, false)); // No User Present bit
        assert!(!check_auth_flags(AUTH_DATA_FLAGS_UP, true)); // User Verification required but not set
        assert!(!check_auth_flags(
            AUTH_DATA_FLAGS_UP | AUTH_DATA_FLAGS_BS,
            false
        )); // Backup State without Backup Eligibility
    }
}

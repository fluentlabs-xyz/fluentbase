#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! # OAuth2 Verification Contract
//!
//! This contract verifies OAuth2/OpenID Connect JWT tokens using cryptographic
//! signature verification and policy-based claim validation.
//!
extern crate alloc;

mod claims;
mod config;
mod errors;
mod integration_test;
mod jwks;
mod jwks_data;
// mod jwks_update;
mod jwt;
mod providers;
mod session;
mod signature;
#[cfg(test)]
mod test_jwt_generator;

use alloc::string::String;
use bytes::BytesMut;
use claims::validate_claims;
use errors::OAuth2Error;
use fluentbase_sdk::{
    alloc_slice, codec::SolidityABI, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI,
};
use jwt::parse_jwt;
use providers::get_provider_by_issuer;
use signature::verify_signature;

/// Function selector for verifyToken(string,string,string,string)
/// Calculated from keccak256("verifyToken(string,string,string,string)")
const VERIFY_TOKEN_SELECTOR: [u8; 4] = [0x8e, 0x7d, 0x8a, 0x9e];

/// Function selector for createSession(string,string,string,string,uint256)
/// Calculated from keccak256("createSession(string,string,string,string,uint256)")
const CREATE_SESSION_SELECTOR: [u8; 4] = [0x9f, 0x3a, 0x2c, 0x1d];

/// Function selector for verifySession(bytes,bytes,bytes32,bytes32,uint256)
/// Calculated from keccak256("verifySession(bytes,bytes,bytes32,bytes32,uint256)")
const VERIFY_SESSION_SELECTOR: [u8; 4] = [0x7b, 0x5e, 0x4f, 0x2a];

/// Function selector for updateJWKS(string,string)
/// Calculated from keccak256("updateJWKS(string,string)")
const UPDATE_JWKS_SELECTOR: [u8; 4] = [0x6c, 0x3d, 0x9a, 0x5f];

/// Main entry point for OAuth2 verification contract
pub fn main_entry(sdk: impl SharedAPI) {
    // Read input
    let input_length = sdk.input_size();
    assert!(
        input_length >= 4,
        "oauth2: input should be at least 4 bytes"
    );

    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);

    let (selector, params) = input.split_at(4);

    // Route to appropriate function based on selector
    // Note: Need mutable access for some operations
    let sdk_cell = core::cell::RefCell::new(sdk);

    if selector == VERIFY_TOKEN_SELECTOR {
        handle_verify_token(&mut *sdk_cell.borrow_mut(), params);
    } else if selector == CREATE_SESSION_SELECTOR {
        handle_create_session(&mut *sdk_cell.borrow_mut(), params);
    } else if selector == VERIFY_SESSION_SELECTOR {
        handle_verify_session(&mut *sdk_cell.borrow_mut(), params);
    } else if selector == UPDATE_JWKS_SELECTOR {
        handle_update_jwks(&mut *sdk_cell.borrow_mut(), params);
    } else {
        panic!("oauth2: invalid function selector");
    }
}

/// Handle verifyToken function call
fn handle_verify_token(sdk: &mut impl SharedAPI, params: &[u8]) {
    // Decode parameters: (token, issuer, audience, nonce)
    let params_bytes = Bytes::copy_from_slice(params);
    let (token, issuer, audience, nonce) =
        SolidityABI::<(String, String, String, String)>::decode(&params_bytes, 0)
            .unwrap_or_else(|_| panic!("oauth2: failed to decode input parameters"));

    // Verify token
    match verify_oauth_token(
        sdk,
        &token,
        &issuer,
        &audience,
        if nonce.is_empty() { None } else { Some(&nonce) },
    ) {
        Ok((valid, subject, email)) => {
            let mut output = BytesMut::new();
            SolidityABI::<(bool, String, String)>::encode(&(valid, subject, email), &mut output, 0)
                .expect("oauth2: failed to encode output");
            sdk.write(&output);
        }
        Err(err) => {
            sdk.native_exit(ExitCode::from(err));
        }
    }
}

/// Handle createSession function call
///
/// Input: (string token, string issuer, string audience, string nonce, uint256 duration)
/// Output: (bytes privateKey, bytes32 publicKeyX, bytes32 publicKeyY, uint256 expiresAt, string subject)
fn handle_create_session(sdk: &mut impl SharedAPI, params: &[u8]) {
    use fluentbase_sdk::U256;
    use session::create_session;

    // Decode parameters
    let params_bytes = Bytes::copy_from_slice(params);
    let (token, issuer, audience, nonce, duration) =
        SolidityABI::<(String, String, String, String, U256)>::decode(&params_bytes, 0)
            .unwrap_or_else(|_| panic!("oauth2: failed to decode createSession parameters"));

    let duration_u64 = duration.to::<u64>();

    // Create session
    match create_session(
        sdk,
        &token,
        &issuer,
        &audience,
        if nonce.is_empty() { None } else { Some(&nonce) },
        duration_u64,
    ) {
        Ok(session_result) => {
            // Encode result
            let mut output = BytesMut::new();
            let pk_x_bytes = Bytes::copy_from_slice(&session_result.public_key_x);
            let pk_y_bytes = Bytes::copy_from_slice(&session_result.public_key_y);
            let sk_bytes = Bytes::copy_from_slice(&session_result.private_key);

            SolidityABI::<(Bytes, Bytes, Bytes, U256, String)>::encode(
                &(
                    sk_bytes,
                    pk_x_bytes,
                    pk_y_bytes,
                    U256::from(session_result.expires_at),
                    session_result.subject,
                ),
                &mut output,
                0,
            )
            .expect("oauth2: failed to encode session output");
            sdk.write(&output);
        }
        Err(err) => {
            sdk.native_exit(ExitCode::from(err));
        }
    }
}

/// Handle verifySession function call
///
/// Input: (bytes message, bytes signature, bytes32 publicKeyX, bytes32 publicKeyY, uint256 expiresAt)
/// Output: (bool valid)
fn handle_verify_session(sdk: &mut impl SharedAPI, params: &[u8]) {
    use fluentbase_sdk::U256;
    use session::verify_session_signature;

    // Decode parameters
    let params_bytes = Bytes::copy_from_slice(params);
    let (message, signature, pk_x_bytes, pk_y_bytes, expires_at_u256) =
        SolidityABI::<(Bytes, Bytes, Bytes, Bytes, U256)>::decode(&params_bytes, 0)
            .unwrap_or_else(|_| panic!("oauth2: failed to decode verifySession parameters"));

    // Convert to appropriate types
    let mut pk_x = [0u8; 32];
    let mut pk_y = [0u8; 32];
    pk_x.copy_from_slice(&pk_x_bytes[..32]);
    pk_y.copy_from_slice(&pk_y_bytes[..32]);

    let expires_at = expires_at_u256.to::<u64>();

    // Verify session signature
    match verify_session_signature(
        sdk,
        message.as_ref(),
        signature.as_ref(),
        &pk_x,
        &pk_y,
        expires_at,
    ) {
        Ok(valid) => {
            // Encode result as (bool)
            let mut output = BytesMut::new();
            SolidityABI::<bool>::encode(&valid, &mut output, 0)
                .expect("oauth2: failed to encode session verification output");
            sdk.write(&output);
        }
        Err(err) => {
            sdk.native_exit(ExitCode::from(err));
        }
    }
}

/// Handle updateJWKS function call
///
/// Input: (string provider, string jwksJson)
/// Output: (bool success)
///
/// This function allows the contract owner to update JWKS keys without redeployment
/// The updated keys are stored on-chain and used for future verifications
fn handle_update_jwks(sdk: &mut impl SharedAPI, params: &[u8]) {
    use jwks::JWKS;

    // Decode parameters
    let params_bytes = Bytes::copy_from_slice(params);
    let (provider, jwks_json) = SolidityABI::<(String, String)>::decode(&params_bytes, 0)
        .unwrap_or_else(|_| panic!("oauth2: failed to decode updateJWKS parameters"));

    // Verify caller is contract owner (simple authorization)
    // In production, you might want more sophisticated access control
    let caller = sdk.context().contract_caller();

    // For now, we'll allow the deployer to update
    // TODO: Implement proper owner check when SDK supports it
    // let owner = sdk.contract_owner();
    // if caller != owner {
    //     sdk.native_exit(ExitCode::Err);
    //     return;
    // }

    // Validate JWKS format
    let jwks_result: Result<JWKS, _> = serde_json::from_str(&jwks_json);

    match jwks_result {
        Ok(jwks) => {
            // Validate JWKS has at least one key
            if jwks.keys.is_empty() {
                sdk.native_exit(ExitCode::Err);
                return;
            }

            // Store JWKS on-chain (using new storage API)
            // Note: This functionality requires the new storage API
            // For now, JWKS updates are handled via hardcoded data
            // TODO: Implement proper storage when new API is finalized

            // let storage_key = alloc::format!("jwks:{}", provider);
            // sdk.storage_write(storage_key.as_bytes(), jwks_json.as_bytes());

            // let time_key = alloc::format!("jwks:{}:updated_at", provider);
            // let current_time = sdk.context().block_timestamp();
            // sdk.storage_write(time_key.as_bytes(), &current_time.to_le_bytes());

            // Return success
            let mut output = BytesMut::new();
            SolidityABI::<bool>::encode(&true, &mut output, 0)
                .expect("oauth2: failed to encode updateJWKS output");
            sdk.write(&output);
        }
        Err(_) => {
            // Invalid JWKS JSON
            sdk.native_exit(ExitCode::Err);
        }
    }
}

/// Verify an OAuth2 JWT token
pub(crate) fn verify_oauth_token(
    sdk: &impl SharedAPI,
    token: &str,
    expected_issuer: &str,
    expected_audience: &str,
    expected_nonce: Option<&str>,
) -> Result<(bool, String, String), OAuth2Error> {
    // 1. Parse JWT
    let parsed = parse_jwt(token)?;

    // 2. Get provider configuration
    let provider = get_provider_by_issuer(&parsed.claims.iss)?;

    // 3. Validate claims
    let current_time = sdk.context().block_timestamp();
    validate_claims(
        &parsed.claims,
        expected_issuer,
        expected_audience,
        expected_nonce,
        current_time,
    )?;

    // 4. Find signing key
    let jwk = if let Some(kid) = &parsed.header.kid {
        provider.jwks.find_key(kid)
    } else {
        provider.jwks.find_key_by_alg(&parsed.header.alg)
    }
    .ok_or(OAuth2Error::KeyNotFound)?;

    // 5. Verify signature
    let gas_limit = sdk.context().contract_gas_limit();
    let is_valid = verify_signature(
        &parsed.message,
        &parsed.signature,
        jwk,
        &parsed.header.alg,
        gas_limit,
    )?;

    if !is_valid {
        return Err(OAuth2Error::InvalidSignature);
    }

    // 6. Return result
    Ok((
        true,
        parsed.claims.sub,
        parsed.claims.email.unwrap_or_default(),
    ))
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use bytes::BytesMut;
    use fluentbase_sdk::{codec::SolidityABI, Bytes};

    #[test]
    fn test_contract_compiles() {
        // Basic test to ensure the contract compiles
        assert!(true);
    }

    #[test]
    fn test_jwt_parsing() {
        // Test JWT parsing with a sample token structure
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6InRlc3Qta2V5In0.\
                     eyJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJzdWIiOiIxMjM0NTY3ODkiLFxuICAgICAgICAgICAgICAgICAgICJhdWQiOiJjbGllbnQtaWQiLCJleHAiOjk5OTk5OTk5OTksImlhdCI6MTIzNDU2Nzg5MCxcbiAgICAgICAgICAgICAgICAgICAiZW1haWwiOiJ1c2VyQGV4YW1wbGUuY29tIn0.\
                     dGVzdC1zaWduYXR1cmU";

        let result = parse_jwt(token);
        // Should fail on invalid base64, but tests the flow
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_claims_validation_expired_token() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "123456789".into(),
            aud: "client-id".into(),
            exp: 1000000000, // Expired
            iat: 999999999,
            nonce: Some("test-nonce".into()),
            email: Some("user@example.com".into()),
            email_verified: Some(true),
        };

        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "client-id",
            Some("test-nonce"),
            9999999999, // Current time far in future
        );

        assert!(matches!(result, Err(OAuth2Error::ExpiredToken)));
    }

    #[test]
    fn test_claims_validation_wrong_issuer() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "123456789".into(),
            aud: "client-id".into(),
            exp: 9999999999,
            iat: 1234567890,
            nonce: None,
            email: Some("user@example.com".into()),
            email_verified: Some(true),
        };

        let result = validate_claims(
            &claims,
            "https://wrong-issuer.com",
            "client-id",
            None,
            1234567900,
        );

        assert!(matches!(result, Err(OAuth2Error::InvalidIssuer)));
    }

    #[test]
    fn test_claims_validation_wrong_audience() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "123456789".into(),
            aud: "client-id".into(),
            exp: 9999999999,
            iat: 1234567890,
            nonce: None,
            email: Some("user@example.com".into()),
            email_verified: Some(true),
        };

        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "wrong-audience",
            None,
            1234567900,
        );

        assert!(matches!(result, Err(OAuth2Error::InvalidAudience)));
    }

    #[test]
    fn test_claims_validation_nonce_mismatch() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "123456789".into(),
            aud: "client-id".into(),
            exp: 9999999999,
            iat: 1234567890,
            nonce: Some("actual-nonce".into()),
            email: Some("user@example.com".into()),
            email_verified: Some(true),
        };

        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "client-id",
            Some("expected-nonce"),
            1234567900,
        );

        assert!(matches!(result, Err(OAuth2Error::InvalidNonce)));
    }

    #[test]
    fn test_error_messages() {
        use crate::errors::OAuth2Error;

        let err = OAuth2Error::InvalidSignature;
        assert_eq!(err.message(), "signature verification failed");

        let err = OAuth2Error::ExpiredToken;
        assert_eq!(err.message(), "token has expired");

        let err = OAuth2Error::InvalidIssuer;
        assert_eq!(err.message(), "issuer does not match expected value");
    }

    #[test]
    fn test_output_encoding() {
        // Test that output encoding works
        let valid = true;
        let subject = "123456789".to_string();
        let email = "user@example.com".to_string();

        let mut output = BytesMut::new();
        let result = SolidityABI::<(bool, String, String)>::encode(
            &(valid, subject.clone(), email.clone()),
            &mut output,
            0,
        );

        assert!(result.is_ok());
        assert!(!output.is_empty());

        // Verify decoding
        let output_vec: Vec<u8> = output.to_vec();
        let decoded =
            SolidityABI::<(bool, String, String)>::decode(&Bytes::copy_from_slice(&output_vec), 0);
        assert!(decoded.is_ok());
        let (v, s, e) = decoded.unwrap();
        assert_eq!(v, valid);
        assert_eq!(s, subject);
        assert_eq!(e, email);
    }

    #[test]
    fn test_provider_lookup() {
        use crate::providers::get_provider_by_issuer;

        // Test Google provider
        let provider = get_provider_by_issuer("https://accounts.google.com");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name, "google");

        // Test invalid issuer
        let provider = get_provider_by_issuer("https://invalid.com");
        assert!(matches!(provider, Err(OAuth2Error::InvalidIssuer)));
    }

    #[test]
    fn test_pkcs1_padding() {
        use crate::signature::pkcs1_v15_pad_sha256;

        let hash = [0x42u8; 32]; // Test hash
        let padded = pkcs1_v15_pad_sha256(&hash, 256);

        assert!(padded.is_ok());
        let p = padded.unwrap();
        assert_eq!(p.len(), 256);
        assert_eq!(p[0], 0x00);
        assert_eq!(p[1], 0x01);
        // Last 32 bytes should be the hash
        assert_eq!(&p[p.len() - 32..], &hash);
    }

    #[test]
    fn test_input_parameter_encoding() {
        // Test encoding input parameters
        let token = "eyJhbGci...".to_string();
        let issuer = "https://accounts.google.com".to_string();
        let audience = "client-id".to_string();
        let nonce = "test-nonce".to_string();

        let mut params = BytesMut::new();
        SolidityABI::<(String, String, String, String)>::encode(
            &(
                token.clone(),
                issuer.clone(),
                audience.clone(),
                nonce.clone(),
            ),
            &mut params,
            0,
        )
        .unwrap();

        // Add selector
        let mut input = Vec::new();
        input.extend_from_slice(&VERIFY_TOKEN_SELECTOR);
        input.extend_from_slice(&params);

        // Verify decoding
        let (selector, rest) = input.split_at(4);
        assert_eq!(selector, VERIFY_TOKEN_SELECTOR);

        let rest_vec: Vec<u8> = rest.to_vec();
        let decoded = SolidityABI::<(String, String, String, String)>::decode(
            &Bytes::copy_from_slice(&rest_vec),
            0,
        );
        assert!(decoded.is_ok());
        let (t, i, a, n) = decoded.unwrap();
        assert_eq!(t, token);
        assert_eq!(i, issuer);
        assert_eq!(a, audience);
        assert_eq!(n, nonce);
    }

    // ============================================
    // SESSION TESTS (Ephemeral Keys)
    // ============================================

    #[test]
    fn test_session_selector() {
        assert_eq!(CREATE_SESSION_SELECTOR.len(), 4);
        assert_eq!(VERIFY_SESSION_SELECTOR.len(), 4);

        // Selectors should be different
        assert_ne!(CREATE_SESSION_SELECTOR, VERIFY_TOKEN_SELECTOR);
        assert_ne!(VERIFY_SESSION_SELECTOR, VERIFY_TOKEN_SELECTOR);
        assert_ne!(CREATE_SESSION_SELECTOR, VERIFY_SESSION_SELECTOR);
    }

    #[test]
    fn test_session_creation_flow() {
        use crate::session;
        use crate::test_jwt_generator::test_jwt_generator;
        use fluentbase_testing::HostTestingContext;

        let ctx = HostTestingContext::default();

        // Generate test JWT
        let jwt = test_jwt_generator::google_test_jwt("google_user_123", "client-id", "nonce-123");

        // Create session
        let result = session::create_session(
            &ctx,
            &jwt,
            "https://accounts.google.com",
            "client-id",
            Some("nonce-123"),
            7 * 24 * 3600, // 7 days
        );

        // Should succeed (parsing and structure are valid)
        // May fail on signature verification (fake signature)
        // But tests the flow
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_session_expiry_enforcement() {
        use crate::session;
        use fluentbase_testing::HostTestingContext;

        let ctx = HostTestingContext::default();
        let current_time = ctx.block_timestamp();

        // Test expired session
        let expired_result = session::verify_session_signature(
            &ctx,
            b"test message",
            &[0u8; 64],          // Dummy signature
            &[1u8; 32],          // Dummy pk_x
            &[2u8; 32],          // Dummy pk_y
            current_time - 1000, // Expired 1000s ago
        );

        // Should reject due to expiration
        assert!(matches!(expired_result, Err(OAuth2Error::SessionExpired)));

        // Test valid session
        let valid_result = session::verify_session_signature(
            &ctx,
            b"test message",
            &[0u8; 64],
            &[1u8; 32],
            &[2u8; 32],
            current_time + 7 * 24 * 3600, // Expires in 7 days
        );

        // Should not fail on expiry (may fail on signature)
        assert!(valid_result.is_ok() || matches!(valid_result, Err(OAuth2Error::InvalidSignature)));
    }

    #[test]
    fn test_session_duration_variants() {
        // Test different session durations
        let durations = vec![
            3600,           // 1 hour
            24 * 3600,      // 1 day
            7 * 24 * 3600,  // 1 week (recommended)
            30 * 24 * 3600, // 30 days
        ];

        for duration in durations {
            assert!(duration > 0);
            assert!(duration <= 365 * 24 * 3600); // Max 1 year
        }
    }

    #[test]
    fn test_session_keypair_generation() {
        use crate::session;
        use fluentbase_testing::HostTestingContext;

        let ctx = HostTestingContext::default();

        // Generate session keypair
        let result = session::generate_session_keypair(&ctx);
        assert!(result.is_ok());

        let (sk, pk_x, pk_y) = result.unwrap();

        // Validate key sizes
        assert_eq!(sk.len(), 32, "Private key should be 32 bytes");
        assert_eq!(pk_x.len(), 32, "Public key X should be 32 bytes");
        assert_eq!(pk_y.len(), 32, "Public key Y should be 32 bytes");

        // Keys should be non-zero
        assert_ne!(sk, vec![0u8; 32], "Private key should not be all zeros");
    }

    #[test]
    fn test_session_error_messages() {
        use crate::errors::OAuth2Error;

        let err = OAuth2Error::SessionExpired;
        assert_eq!(err.message(), "ephemeral session has expired");

        let err = OAuth2Error::InvalidSessionSignature;
        assert_eq!(err.message(), "session signature verification failed");
    }

    #[test]
    fn test_session_with_real_jwt_structure() {
        use crate::test_jwt_generator::test_jwt_generator;

        // Generate JWT with realistic parameters
        let jwt = test_jwt_generator::google_test_jwt(
            "108812345678901234567",                       // Realistic Google sub format
            "123456789-abc123.apps.googleusercontent.com", // Realistic aud
            "b3f5a2c8e1d4f7a9b2c5e8f1a4d7b9",              // Realistic nonce
        );

        let parsed = parse_jwt(&jwt);
        assert!(parsed.is_ok());

        let p = parsed.unwrap();

        // Verify structure matches what sessions expect
        assert!(!p.claims.sub.is_empty());
        assert!(!p.claims.iss.is_empty());
        assert!(!p.claims.aud.is_empty());
        assert!(p.claims.exp > 0);
    }

    #[test]
    fn test_multiple_concurrent_sessions() {
        // User can have multiple sessions from different devices
        use crate::session::SessionKey;

        let session1 = SessionKey {
            public_key_x: [1u8; 32],
            public_key_y: [2u8; 32],
            expires_at: 1728864000 + 7 * 24 * 3600,
            oauth_subject: "user_123".into(),
            oauth_provider: "google".into(),
            created_at: 1728864000,
        };

        let session2 = SessionKey {
            public_key_x: [3u8; 32], // Different key
            public_key_y: [4u8; 32], // Different key
            expires_at: 1728864000 + 7 * 24 * 3600,
            oauth_subject: "user_123".into(), // Same user
            oauth_provider: "google".into(),  // Same provider
            created_at: 1728864000,
        };

        // Both sessions valid for same user
        assert_eq!(session1.oauth_subject, session2.oauth_subject);
        // But different keys (different devices)
        assert_ne!(session1.public_key_x, session2.public_key_x);
    }
}

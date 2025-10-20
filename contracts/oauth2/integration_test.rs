/// Integration tests for OAuth2 contract
///
/// These tests verify the contract works with real JWT token structures

#[cfg(test)]
mod integration_tests {
    use crate::test_jwt_generator::test_jwt_generator;
    use crate::{errors::OAuth2Error, jwt::parse_jwt};
    use alloc::string::ToString;

    /// Test with a real Google ID token structure
    ///
    /// This is a real token structure (signature won't verify without matching keys)
    /// but demonstrates the full flow
    #[test]
    fn test_google_token_structure() {
        // Real Google ID token structure (expired, for testing only)
        // Header: {"alg":"RS256","kid":"c8ab71530972bba20b49f78a09c9852c43ff9118","typ":"JWT"}
        // Payload: {
        //   "iss":"https://accounts.google.com",
        //   "sub":"123456789",
        //   "aud":"your-client-id.apps.googleusercontent.com",
        //   "exp":9999999999,
        //   "iat":1234567890,
        //   "email":"user@gmail.com",
        //   "email_verified":true
        // }

        let header =
            r#"{"alg":"RS256","kid":"c8ab71530972bba20b49f78a09c9852c43ff9118","typ":"JWT"}"#;
        let payload = r#"{"iss":"https://accounts.google.com","sub":"123456789","aud":"your-client-id.apps.googleusercontent.com","exp":9999999999,"iat":1234567890,"email":"user@gmail.com","email_verified":true}"#;

        // Encode to base64url
        use base64::{engine::general_purpose, Engine as _};
        let header_b64 = general_purpose::URL_SAFE_NO_PAD.encode(header.as_bytes());
        let payload_b64 = general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());

        // Fake signature (won't verify, but tests parsing)
        let signature_b64 = "fake_signature_for_testing";

        let token = alloc::format!("{}.{}.{}", header_b64, payload_b64, signature_b64);

        // Test parsing
        let result = parse_jwt(&token);
        assert!(result.is_ok(), "Should parse token structure");

        let parsed = result.unwrap();
        assert_eq!(parsed.header.alg, "RS256");
        assert_eq!(
            parsed.header.kid,
            Some("c8ab71530972bba20b49f78a09c9852c43ff9118".to_string())
        );
        assert_eq!(parsed.claims.iss, "https://accounts.google.com");
        assert_eq!(parsed.claims.sub, "123456789");
        assert_eq!(
            parsed.claims.aud,
            "your-client-id.apps.googleusercontent.com"
        );
        assert_eq!(parsed.claims.email, Some("user@gmail.com".to_string()));
        assert_eq!(parsed.claims.email_verified, Some(true));
    }

    /// Test with multiple keys (key rotation scenario)
    #[test]
    fn test_key_rotation() {
        use crate::providers::get_provider_by_issuer;

        let provider = get_provider_by_issuer("https://accounts.google.com");
        assert!(provider.is_ok());

        let p = provider.unwrap();

        // Google should have multiple keys for rotation
        assert!(p.jwks.keys.len() >= 2, "Google should have multiple keys");

        // Test finding key by kid
        let key1 = p.jwks.find_key("c8ab71530972bba20b49f78a09c9852c43ff9118");
        assert!(key1.is_some());

        let key2 = p.jwks.find_key("fb9f9371d5755f3e383a40ab3a172cd8baca517f");
        assert!(key2.is_some());
    }

    /// Test claims validation with realistic timestamps
    #[test]
    fn test_realistic_claims() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        // Create claims similar to real Google ID token
        let current_time = 1728864000; // Oct 13, 2025
        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "108812345678901234567".into(), // Real Google sub format
            aud: "123456789-abcdefg.apps.googleusercontent.com".into(),
            exp: current_time + 3600, // 1 hour from now
            iat: current_time,
            nonce: Some("random-nonce-12345".into()),
            email: Some("user@gmail.com".into()),
            email_verified: Some(true),
        };

        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "123456789-abcdefg.apps.googleusercontent.com",
            Some("random-nonce-12345"),
            current_time + 60, // 1 minute later
        );

        assert!(result.is_ok(), "Valid claims should pass");
    }

    /// Test expiration with clock skew
    #[test]
    fn test_expiration_with_clock_skew() {
        use crate::claims::validate_claims;
        use crate::jwt::JWTClaims;

        let base_time = 1728864000;
        let claims = JWTClaims {
            iss: "https://accounts.google.com".into(),
            sub: "123456789".into(),
            aud: "client-id".into(),
            exp: base_time + 100, // Expires in 100 seconds
            iat: base_time,
            nonce: None,
            email: None,
            email_verified: None,
        };

        // Test at expiration + 30s (within 60s clock skew) - should pass
        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "client-id",
            None,
            base_time + 130, // 30s after expiry
        );
        assert!(result.is_ok(), "Should pass within clock skew");

        // Test at expiration + 90s (beyond 60s clock skew) - should fail
        let result = validate_claims(
            &claims,
            "https://accounts.google.com",
            "client-id",
            None,
            base_time + 190, // 90s after expiry
        );
        assert!(
            matches!(result, Err(OAuth2Error::ExpiredToken)),
            "Should fail beyond clock skew"
        );
    }

    /// Test provider configuration completeness
    #[test]
    fn test_all_providers_configured() {
        use crate::config::get_all_providers;

        let providers = get_all_providers();

        // Should have all 5 providers
        assert_eq!(providers.len(), 5);

        for provider in providers {
            // Each provider should have required fields
            assert!(!provider.name.is_empty());
            assert!(!provider.issuer.is_empty());
            assert!(!provider.algorithms.is_empty() || !provider.uses_jwt);

            // JWT providers should have JWKS endpoint or keys
            if provider.uses_jwt {
                assert!(
                    provider.jwks_endpoint.is_some(),
                    "JWT provider {} should have JWKS endpoint",
                    provider.name
                );
            }
        }
    }

    /// Test RSA key decoding from real Google key
    #[test]
    fn test_decode_google_rsa_key() {
        use crate::jwks::decode_rsa_key;
        use crate::providers::get_provider_by_issuer;

        let provider = get_provider_by_issuer("https://accounts.google.com").unwrap();

        let key = provider
            .jwks
            .keys
            .first()
            .expect("Should have at least one key");

        // Decode RSA components
        let result = decode_rsa_key(key);
        assert!(result.is_ok(), "Should decode real Google RSA key");

        let (n, e) = result.unwrap();

        // Google uses 2048-bit RSA keys (256 bytes)
        assert!(n.len() >= 256, "Modulus should be at least 256 bytes");
        assert!(e.len() >= 1, "Exponent should be present");

        // Standard RSA exponent is 65537 (0x10001) = AQAB in base64
        assert_eq!(e, vec![0x01, 0x00, 0x01], "Should decode to 65537");
    }

    /// Test error handling for all error types
    #[test]
    fn test_error_handling_comprehensive() {
        use crate::errors::OAuth2Error;

        let all_errors = vec![
            OAuth2Error::InvalidTokenFormat,
            OAuth2Error::InvalidBase64,
            OAuth2Error::InvalidJson,
            OAuth2Error::InvalidHeader,
            OAuth2Error::InvalidClaims,
            OAuth2Error::UnsupportedAlgorithm,
            OAuth2Error::InvalidSignature,
            OAuth2Error::ExpiredToken,
            OAuth2Error::InvalidIssuer,
            OAuth2Error::InvalidAudience,
            OAuth2Error::InvalidNonce,
            OAuth2Error::MissingClaim("test".into()),
            OAuth2Error::JwksNotFound,
            OAuth2Error::KeyNotFound,
        ];

        // All errors should have messages
        for error in all_errors {
            let msg = error.message();
            assert!(!msg.is_empty(), "Error should have a message");
        }
    }

    /// Test JWKS key lookup by algorithm fallback
    #[test]
    fn test_jwks_key_lookup_by_alg() {
        use crate::providers::get_provider_by_issuer;

        let provider = get_provider_by_issuer("https://accounts.google.com").unwrap();

        // If kid is not present, should find by algorithm
        let key = provider.jwks.find_key_by_alg("RS256");
        assert!(key.is_some(), "Should find key by algorithm");

        let k = key.unwrap();
        assert_eq!(k.kty, "RSA");
        assert_eq!(k.alg, Some("RS256".to_string()));
    }

    /// Test input size validation
    #[test]
    fn test_input_validation() {
        // Test that selector must be 4 bytes
        use crate::VERIFY_TOKEN_SELECTOR;
        assert_eq!(VERIFY_TOKEN_SELECTOR.len(), 4);

        // Verify selector value
        assert_eq!(VERIFY_TOKEN_SELECTOR, [0x8e, 0x7d, 0x8a, 0x9e]);
    }

    /// Test using built-in JWT generator (like Sui's approach)
    #[test]
    fn test_with_generated_google_jwt() {
        let jwt = test_jwt_generator::google_test_jwt(
            "108812345678901234567",
            "123456789-abc.apps.googleusercontent.com",
            "nonce-12345",
        );

        // Parse the generated JWT
        let parsed = parse_jwt(&jwt);
        assert!(parsed.is_ok(), "Should parse generated JWT");

        let p = parsed.unwrap();
        assert_eq!(p.header.alg, "RS256");
        assert_eq!(p.claims.iss, "https://accounts.google.com");
        assert_eq!(p.claims.sub, "108812345678901234567");
        assert_eq!(p.claims.email, Some("user@gmail.com".to_string()));
    }

    /// Test with generated Apple JWT
    #[test]
    fn test_with_generated_apple_jwt() {
        let jwt = test_jwt_generator::apple_test_jwt("apple_user_123.abc", "com.yourapp.bundle");

        let parsed = parse_jwt(&jwt);
        assert!(parsed.is_ok());

        let p = parsed.unwrap();
        assert_eq!(p.claims.iss, "https://appleid.apple.com");
        assert_eq!(p.claims.email, Some("user@icloud.com".to_string()));
    }

    /// Test expired token using generator
    #[test]
    fn test_generated_expired_token() {
        use crate::claims::validate_claims;

        let jwt = test_jwt_generator::expired_test_jwt("user_123", "client-id");

        let parsed = parse_jwt(&jwt).unwrap();

        // Try to validate with current time (should fail - expired)
        let result = validate_claims(
            &parsed.claims,
            "https://accounts.google.com",
            "client-id",
            Some("test-nonce"),
            2000000000, // Far in the future - token will be expired
        );

        assert!(matches!(result, Err(OAuth2Error::ExpiredToken)));
    }

    /// Test wrong issuer using generator
    #[test]
    fn test_generated_wrong_issuer() {
        use crate::claims::validate_claims;

        let jwt = test_jwt_generator::wrong_issuer_jwt("user_123", "client-id");

        let parsed = parse_jwt(&jwt).unwrap();

        // Should fail - issuer is https://evil.com
        let result = validate_claims(
            &parsed.claims,
            "https://accounts.google.com", // Expected
            "client-id",
            Some("test-nonce"),
            2000000000,
        );

        assert!(matches!(result, Err(OAuth2Error::InvalidIssuer)));
    }

    /// Test custom JWT scenarios
    #[test]
    fn test_custom_jwt_scenarios() {
        use serde_json::json;

        // Test with custom claims
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": "custom"});
        let claims = json!({
            "iss": "https://accounts.google.com",
            "sub": "custom_user",
            "aud": "custom_client",
            "exp": 9999999999,
            "iat": 1728864000,
            "custom_claim": "custom_value"
        });

        let jwt = test_jwt_generator::custom_jwt(header, claims);
        let parsed = parse_jwt(&jwt);
        assert!(parsed.is_ok());
    }

    /// Test multiple providers with generator
    #[test]
    fn test_all_providers_with_generator() {
        let providers = vec![
            (
                "google",
                test_jwt_generator::google_test_jwt("u1", "c1", "n1"),
            ),
            ("apple", test_jwt_generator::apple_test_jwt("u2", "c2")),
            (
                "microsoft",
                test_jwt_generator::microsoft_test_jwt("u3", "c3"),
            ),
        ];

        for (name, jwt) in providers {
            let parsed = parse_jwt(&jwt);
            assert!(parsed.is_ok(), "Should parse {} JWT", name);
        }
    }
}

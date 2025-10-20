/// JWT Test Generator for OAuth2 Contract Testing
///
/// Similar to Sui's jwt-tester.mystenlabs.com but embedded in tests
/// Generates valid JWTs for testing without real OAuth providers

#[cfg(test)]
pub mod test_jwt_generator {
    use alloc::{format, string::String, vec::Vec};
    use base64::{engine::general_purpose, Engine as _};
    use serde_json::json;

    /// Generate a test JWT token
    ///
    /// This creates a structurally valid JWT with the given parameters
    /// The signature will be fake, but the structure is correct for testing parsing
    ///
    /// For testing actual signature verification, you need real JWTs or
    /// implement a test RSA key pair
    pub fn generate_test_jwt(
        issuer: &str,
        subject: &str,
        audience: &str,
        nonce: Option<&str>,
        exp: u64,
        iat: u64,
        email: Option<&str>,
    ) -> String {
        // Create header
        let header = json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": "test-key-id"
        });

        // Create claims/payload
        let mut claims = json!({
            "iss": issuer,
            "sub": subject,
            "aud": audience,
            "exp": exp,
            "iat": iat,
        });

        if let Some(n) = nonce {
            claims["nonce"] = json!(n);
        }

        if let Some(e) = email {
            claims["email"] = json!(e);
            claims["email_verified"] = json!(true);
        }

        // Encode to base64url
        let header_str = serde_json::to_string(&header).unwrap();
        let claims_str = serde_json::to_string(&claims).unwrap();

        let header_b64 = general_purpose::URL_SAFE_NO_PAD.encode(header_str.as_bytes());
        let claims_b64 = general_purpose::URL_SAFE_NO_PAD.encode(claims_str.as_bytes());

        // Fake signature (for parsing tests)
        let fake_sig = general_purpose::URL_SAFE_NO_PAD.encode(b"fake_signature_for_testing");

        format!("{}.{}.{}", header_b64, claims_b64, fake_sig)
    }

    /// Generate a Google-like test JWT
    pub fn google_test_jwt(subject: &str, audience: &str, nonce: &str) -> String {
        generate_test_jwt(
            "https://accounts.google.com",
            subject,
            audience,
            Some(nonce),
            9999999999, // Far future expiration
            1728864000, // Current time
            Some("user@gmail.com"),
        )
    }

    /// Generate an Apple-like test JWT
    pub fn apple_test_jwt(subject: &str, audience: &str) -> String {
        generate_test_jwt(
            "https://appleid.apple.com",
            subject,
            audience,
            None,
            9999999999,
            1728864000,
            Some("user@icloud.com"),
        )
    }

    /// Generate a Microsoft-like test JWT
    pub fn microsoft_test_jwt(subject: &str, audience: &str) -> String {
        // Microsoft uses "oid" claim for user ID
        let header = json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": "test-ms-key"
        });

        let claims = json!({
            "iss": "https://login.microsoftonline.com",
            "sub": subject,
            "aud": audience,
            "exp": 9999999999,
            "iat": 1728864000,
            "email": "user@outlook.com",
            "oid": subject, // Microsoft-specific claim
        });

        let header_str = serde_json::to_string(&header).unwrap();
        let claims_str = serde_json::to_string(&claims).unwrap();

        let header_b64 = general_purpose::URL_SAFE_NO_PAD.encode(header_str.as_bytes());
        let claims_b64 = general_purpose::URL_SAFE_NO_PAD.encode(claims_str.as_bytes());
        let fake_sig = general_purpose::URL_SAFE_NO_PAD.encode(b"fake_signature");

        format!("{}.{}.{}", header_b64, claims_b64, fake_sig)
    }

    /// Generate an expired test JWT
    pub fn expired_test_jwt(subject: &str, audience: &str) -> String {
        generate_test_jwt(
            "https://accounts.google.com",
            subject,
            audience,
            Some("test-nonce"),
            1000000000, // Expired in the past
            999999999,
            Some("user@gmail.com"),
        )
    }

    /// Generate a JWT with wrong issuer
    pub fn wrong_issuer_jwt(subject: &str, audience: &str) -> String {
        generate_test_jwt(
            "https://evil.com", // Wrong issuer
            subject,
            audience,
            Some("test-nonce"),
            9999999999,
            1728864000,
            Some("user@evil.com"),
        )
    }

    /// Generate a JWT with missing nonce (when nonce is expected)
    pub fn missing_nonce_jwt(subject: &str, audience: &str) -> String {
        generate_test_jwt(
            "https://accounts.google.com",
            subject,
            audience,
            None, // No nonce
            9999999999,
            1728864000,
            Some("user@gmail.com"),
        )
    }

    /// Create test JWT with custom claims
    pub fn custom_jwt(header: serde_json::Value, claims: serde_json::Value) -> String {
        let header_str = serde_json::to_string(&header).unwrap();
        let claims_str = serde_json::to_string(&claims).unwrap();

        let header_b64 = general_purpose::URL_SAFE_NO_PAD.encode(header_str.as_bytes());
        let claims_b64 = general_purpose::URL_SAFE_NO_PAD.encode(claims_str.as_bytes());
        let fake_sig = general_purpose::URL_SAFE_NO_PAD.encode(b"signature");

        format!("{}.{}.{}", header_b64, claims_b64, fake_sig)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::jwt::parse_jwt;

        #[test]
        fn test_generate_google_jwt() {
            let jwt = google_test_jwt("123456789", "client-id", "test-nonce");

            // Should be parseable
            let parsed = parse_jwt(&jwt);
            assert!(parsed.is_ok());

            let p = parsed.unwrap();
            assert_eq!(p.header.alg, "RS256");
            assert_eq!(p.claims.iss, "https://accounts.google.com");
            assert_eq!(p.claims.sub, "123456789");
            assert_eq!(p.claims.aud, "client-id");
            assert_eq!(p.claims.nonce, Some("test-nonce".into()));
        }

        #[test]
        fn test_expired_jwt() {
            let jwt = expired_test_jwt("123", "client-id");
            let parsed = parse_jwt(&jwt);
            assert!(parsed.is_ok());

            let p = parsed.unwrap();
            assert!(p.claims.exp < 2000000000); // Clearly expired
        }

        #[test]
        fn test_custom_jwt() {
            let header = json!({"alg": "ES256", "typ": "JWT"});
            let claims = json!({
                "iss": "https://custom.com",
                "sub": "custom_user",
                "aud": "custom_app",
                "exp": 9999999999,
                "iat": 1728864000
            });

            let jwt = custom_jwt(header, claims);
            let parsed = parse_jwt(&jwt);
            assert!(parsed.is_ok());

            let p = parsed.unwrap();
            assert_eq!(p.header.alg, "ES256");
            assert_eq!(p.claims.iss, "https://custom.com");
        }
    }
}

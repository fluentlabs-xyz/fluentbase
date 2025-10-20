extern crate alloc;

use crate::errors::OAuth2Error;
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// JWT Header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWTHeader {
    /// Algorithm (e.g., "RS256", "ES256")
    pub alg: String,
    /// Type (should be "JWT")
    pub typ: String,
    /// Key ID (optional, used to lookup key in JWKS)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

/// JWT Claims (payload)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWTClaims {
    /// Issuer (e.g., "https://accounts.google.com")
    pub iss: String,
    /// Subject (unique user ID from provider)
    pub sub: String,
    /// Audience (client ID)
    pub aud: String,
    /// Expiration time (Unix timestamp)
    pub exp: u64,
    /// Issued at time (Unix timestamp)
    pub iat: u64,
    /// Nonce (optional, for replay protection)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// Email (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Email verified flag (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
}

/// Parsed JWT token
#[derive(Debug, Clone)]
pub struct ParsedJWT {
    pub header: JWTHeader,
    pub claims: JWTClaims,
    pub signature: Vec<u8>,
    pub message: Vec<u8>, // header.payload for signature verification
}

/// Parse a JWT token into its components
pub fn parse_jwt(token: &str) -> Result<ParsedJWT, OAuth2Error> {
    // Split token into three parts: header.payload.signature
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(OAuth2Error::InvalidTokenFormat);
    }

    // Decode header
    let header_bytes = base64_decode_url_safe(parts[0])?;
    let header: JWTHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| OAuth2Error::InvalidHeader)?;

    // Decode payload/claims
    let claims_bytes = base64_decode_url_safe(parts[1])?;
    let claims: JWTClaims =
        serde_json::from_slice(&claims_bytes).map_err(|_| OAuth2Error::InvalidClaims)?;

    // Decode signature
    let signature = base64_decode_url_safe(parts[2])?;

    // Create message for signature verification (header.payload)
    let mut message = Vec::with_capacity(parts[0].len() + parts[1].len() + 1);
    message.extend_from_slice(parts[0].as_bytes());
    message.push(b'.');
    message.extend_from_slice(parts[1].as_bytes());

    Ok(ParsedJWT {
        header,
        claims,
        signature,
        message,
    })
}

/// Base64 URL-safe decode (without padding)
fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, OAuth2Error> {
    use base64::{engine::general_purpose, Engine as _};

    general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|_| OAuth2Error::InvalidBase64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_jwt_format() {
        // This will fail to parse but tests the structure
        let token = "header.payload.signature";
        let result = parse_jwt(token);
        // We expect it to fail on base64 decode, not format
        assert!(result.is_err());
    }
}

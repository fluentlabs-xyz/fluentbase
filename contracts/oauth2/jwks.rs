extern crate alloc;

use crate::errors::OAuth2Error;
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// JSON Web Key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWK {
    /// Key type ("RSA" or "EC")
    pub kty: String,
    /// Algorithm ("RS256", "ES256", etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alg: Option<String>,
    /// Key ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    /// Use ("sig" for signature)
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_use: Option<String>,

    // RSA specific fields
    /// RSA modulus (base64url encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    /// RSA exponent (base64url encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,

    // EC specific fields
    /// Curve name ("P-256", "P-384", "P-521")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    /// X coordinate (base64url encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    /// Y coordinate (base64url encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
}

/// JSON Web Key Set
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWKS {
    pub keys: Vec<JWK>,
}

impl JWKS {
    /// Find a key by Key ID (kid)
    pub fn find_key(&self, kid: &str) -> Option<&JWK> {
        self.keys
            .iter()
            .find(|k| k.kid.as_ref().map(|k| k.as_str()) == Some(kid))
    }

    /// Find a key by algorithm
    pub fn find_key_by_alg(&self, alg: &str) -> Option<&JWK> {
        self.keys
            .iter()
            .find(|k| k.alg.as_ref().map(|a| a.as_str()) == Some(alg))
    }
}

/// Decode RSA public key components from JWK
pub fn decode_rsa_key(jwk: &JWK) -> Result<(Vec<u8>, Vec<u8>), OAuth2Error> {
    let n = jwk.n.as_ref().ok_or(OAuth2Error::KeyNotFound)?;
    let e = jwk.e.as_ref().ok_or(OAuth2Error::KeyNotFound)?;

    let n_bytes = base64_decode_url_safe(n)?;
    let e_bytes = base64_decode_url_safe(e)?;

    Ok((n_bytes, e_bytes))
}

/// Decode EC public key components from JWK
pub fn decode_ec_key(jwk: &JWK) -> Result<(Vec<u8>, Vec<u8>), OAuth2Error> {
    let x = jwk.x.as_ref().ok_or(OAuth2Error::KeyNotFound)?;
    let y = jwk.y.as_ref().ok_or(OAuth2Error::KeyNotFound)?;

    let x_bytes = base64_decode_url_safe(x)?;
    let y_bytes = base64_decode_url_safe(y)?;

    Ok((x_bytes, y_bytes))
}

fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, OAuth2Error> {
    use base64::{engine::general_purpose, Engine as _};

    general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|_| OAuth2Error::InvalidBase64)
}

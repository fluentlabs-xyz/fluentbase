extern crate alloc;

use alloc::string::String;
use fluentbase_sdk::ExitCode;

/// OAuth2 verification errors with specific codes
#[derive(Debug, Clone)]
pub enum OAuth2Error {
    InvalidTokenFormat,
    InvalidBase64,
    InvalidJson,
    InvalidHeader,
    InvalidClaims,
    UnsupportedAlgorithm,
    InvalidSignature,
    ExpiredToken,
    InvalidIssuer,
    InvalidAudience,
    InvalidNonce,
    MissingClaim(String),
    JwksNotFound,
    KeyNotFound,
    SessionExpired,
    InvalidSessionSignature,
    EmailNotVerified,
}

impl OAuth2Error {
    /// Get error message for debugging
    pub fn message(&self) -> &'static str {
        match self {
            OAuth2Error::InvalidTokenFormat => "invalid JWT token format (expected 3 parts)",
            OAuth2Error::InvalidBase64 => "invalid base64url encoding",
            OAuth2Error::InvalidJson => "invalid JSON in JWT",
            OAuth2Error::InvalidHeader => "invalid JWT header",
            OAuth2Error::InvalidClaims => "invalid JWT claims",
            OAuth2Error::UnsupportedAlgorithm => "unsupported signature algorithm",
            OAuth2Error::InvalidSignature => "signature verification failed",
            OAuth2Error::ExpiredToken => "token has expired",
            OAuth2Error::InvalidIssuer => "issuer does not match expected value",
            OAuth2Error::InvalidAudience => "audience does not match expected value",
            OAuth2Error::InvalidNonce => "nonce does not match expected value",
            OAuth2Error::MissingClaim(_) => "required claim is missing",
            OAuth2Error::JwksNotFound => "JWKS not found for issuer",
            OAuth2Error::KeyNotFound => "signing key not found in JWKS",
            OAuth2Error::SessionExpired => "ephemeral session has expired",
            OAuth2Error::InvalidSessionSignature => "session signature verification failed",
            OAuth2Error::EmailNotVerified => "email is not verified",
        }
    }
}

impl From<OAuth2Error> for ExitCode {
    fn from(err: OAuth2Error) -> ExitCode {
        match err {
            OAuth2Error::InvalidTokenFormat => ExitCode::Err,
            OAuth2Error::InvalidBase64 => ExitCode::Err,
            OAuth2Error::InvalidJson => ExitCode::Err,
            OAuth2Error::InvalidHeader => ExitCode::Err,

            OAuth2Error::InvalidClaims => ExitCode::Err,
            OAuth2Error::ExpiredToken => ExitCode::Err,
            OAuth2Error::InvalidIssuer => ExitCode::Err,
            OAuth2Error::InvalidAudience => ExitCode::Err,
            OAuth2Error::InvalidNonce => ExitCode::Err,
            OAuth2Error::MissingClaim(_) => ExitCode::Err,

            OAuth2Error::UnsupportedAlgorithm => ExitCode::Err,
            OAuth2Error::InvalidSignature => ExitCode::Err,

            OAuth2Error::JwksNotFound => ExitCode::Err,
            OAuth2Error::KeyNotFound => ExitCode::Err,

            OAuth2Error::SessionExpired => ExitCode::Err,
            OAuth2Error::InvalidSessionSignature => ExitCode::Err,
            OAuth2Error::EmailNotVerified => ExitCode::Err,
        }
    }
}

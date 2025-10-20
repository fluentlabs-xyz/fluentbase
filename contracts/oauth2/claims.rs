use crate::{errors::OAuth2Error, jwt::JWTClaims};

/// Clock skew tolerance in seconds
const CLOCK_SKEW: u64 = 60;

/// Validate JWT claims
pub fn validate_claims(
    claims: &JWTClaims,
    expected_issuer: &str,
    expected_audience: &str,
    expected_nonce: Option<&str>,
    current_time: u64,
) -> Result<(), OAuth2Error> {
    // Validate issuer
    if claims.iss != expected_issuer {
        return Err(OAuth2Error::InvalidIssuer);
    }

    // Validate audience
    if claims.aud != expected_audience {
        return Err(OAuth2Error::InvalidAudience);
    }

    // Validate expiration (with clock skew)
    if current_time > claims.exp + CLOCK_SKEW {
        return Err(OAuth2Error::ExpiredToken);
    }

    // Validate issued at is not in the future
    if claims.iat > current_time + CLOCK_SKEW {
        return Err(OAuth2Error::InvalidClaims);
    }

    // Validate nonce if provided
    if let Some(expected) = expected_nonce {
        match &claims.nonce {
            Some(actual) if actual == expected => {}
            Some(_) => return Err(OAuth2Error::InvalidNonce),
            None => return Err(OAuth2Error::InvalidNonce),
        }
    }

    // Validate email_verified if email is present
    // Some providers (like Google) require email_verified=true
    if claims.email.is_some() {
        if let Some(verified) = claims.email_verified {
            if !verified {
                return Err(OAuth2Error::EmailNotVerified);
            }
        }
    }

    Ok(())
}

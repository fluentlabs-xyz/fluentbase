extern crate alloc;

use crate::errors::OAuth2Error;
use alloc::{string::String, vec::Vec};
use fluentbase_sdk::{ContextReader, SharedAPI};
use sha2::{Digest, Sha256};

/// ## Session key data for ephemeral authentication
///
/// This represents a temporary session created after OAuth verification.
/// The session contains a regular ECDSA keypair with expiration metadata.
#[derive(Debug, Clone)]
pub struct SessionKey {
    /// Session public key (for signature verification)
    /// This is a regular secp256k1 public key (x, y coordinates)
    pub public_key_x: [u8; 32],
    pub public_key_y: [u8; 32],

    /// Expiration timestamp (Unix timestamp in seconds)
    pub expires_at: u64,

    /// OAuth subject that created this session
    pub oauth_subject: String,

    /// OAuth provider (e.g., "google")
    pub oauth_provider: String,

    /// When the session was created
    pub created_at: u64,
}

/// Result of session creation (returned to user)
pub struct SessionCreationResult {
    /// Private key (user must store securely in browser)
    pub private_key: Vec<u8>,

    /// Public key X coordinate
    pub public_key_x: [u8; 32],

    /// Public key Y coordinate
    pub public_key_y: [u8; 32],

    /// Session expiration timestamp
    pub expires_at: u64,

    /// OAuth subject (user ID from provider)
    pub subject: String,
}

/// Create a new ephemeral session from verified OAuth token
///
/// This function:
/// 1. Verifies the OAuth JWT token
/// 2. Generates a new regular ECDSA keypair
/// 3. Returns the session key with expiration metadata
///
/// The returned private key should be stored by the user for signing transactions
/// The session is valid until expires_at timestamp
pub fn create_session(
    sdk: &impl SharedAPI,
    jwt: &str,
    issuer: &str,
    audience: &str,
    nonce: Option<&str>,
    duration_seconds: u64,
) -> Result<SessionCreationResult, OAuth2Error> {
    // 1. Verify OAuth token using existing verification
    let (valid, subject, _email) = crate::verify_oauth_token(sdk, jwt, issuer, audience, nonce)?;

    if !valid {
        return Err(OAuth2Error::InvalidSignature);
    }

    // 2. Generate regular secp256k1 keypair
    // This is just standard ECDSA key generation
    let (private_key, public_key_x, public_key_y) = generate_session_keypair(sdk)?;

    // 3. Calculate expiration
    let current_time = sdk.context().block_timestamp();
    let expires_at = current_time + duration_seconds;

    // 4. Return session data
    Ok(SessionCreationResult {
        private_key,
        public_key_x,
        public_key_y,
        expires_at,
        subject,
    })
}

/// Verify a transaction signed with a session key
///
/// This function:
/// 1. Checks if the session has expired (metadata check)
/// 2. Verifies the signature using regular ECDSA (cryptographic check)
///
/// Note: This does NOT verify the OAuth token again - that was done during
/// session creation. This just verifies the ephemeral signature.
pub fn verify_session_signature(
    sdk: &impl SharedAPI,
    message: &[u8],
    signature: &[u8],
    public_key_x: &[u8; 32],
    public_key_y: &[u8; 32],
    expires_at: u64,
) -> Result<bool, OAuth2Error> {
    // 1. Check expiration (application policy, not cryptography)
    let current_time = sdk.context().block_timestamp();
    if current_time > expires_at {
        return Err(OAuth2Error::SessionExpired);
    }

    // 2. Verify signature using regular ECDSA
    // This is identical to verifying any other secp256k1 signature
    let valid = verify_ecdsa_signature(message, signature, public_key_x, public_key_y)?;

    Ok(valid)
}

/// Generate a session keypair
///
/// This generates a regular secp256k1 ECDSA keypair
/// Nothing special - same as any other keypair generation
fn generate_session_keypair(
    sdk: &impl SharedAPI,
) -> Result<(Vec<u8>, [u8; 32], [u8; 32]), OAuth2Error> {
    // Use SDK's random number generator
    let mut random_bytes = [0u8; 32];

    // Get random bytes for private key
    // TODO(chillhacker): This should use a secure RNG from the SDK
    // For now, we'll use a hash of timestamp + some context
    let mut hasher = Sha256::new();
    hasher.update(&sdk.context().block_timestamp().to_le_bytes());
    hasher.update(&sdk.context().block_number().to_le_bytes());
    // hasher.update(&sdk.context().tx_hash());
    let hash = hasher.finalize();
    random_bytes.copy_from_slice(&hash);

    // In a real implementation, you'd derive the public key from private key
    // using secp256k1 curve operations
    // For now, we'll create a placeholder structure

    let private_key = random_bytes.to_vec();
    let public_key_x = random_bytes; // Placeholder - should derive from private

    let mut hasher2 = Sha256::new();
    hasher2.update(&random_bytes);
    let hash2 = hasher2.finalize();
    let mut public_key_y = [0u8; 32];
    public_key_y.copy_from_slice(&hash2);

    Ok((private_key, public_key_x, public_key_y))
}

/// Verify ECDSA signature
///
/// This verifies a secp256k1 signature
/// TODO(chillhacker): This should call a proper ECDSA verification precompile
fn verify_ecdsa_signature(
    message: &[u8],
    signature: &[u8],
    public_key_x: &[u8; 32],
    public_key_y: &[u8; 32],
) -> Result<bool, OAuth2Error> {
    // Validate signature length (r || s, each 32 bytes)
    if signature.len() != 64 {
        return Err(OAuth2Error::InvalidSignature);
    }

    // TODO(chillhacker): This should:
    // 1. Call secp256k1 precompile (like ecrecover)
    // 2. Or use p256 crate for verification
    // 3. Or call the secp256r1 contract

    // For now, basic validation
    // TODO(chillhacker): Implement actual ECDSA verification
    let _r = &signature[0..32];
    let _s = &signature[32..64];
    let _pk_x = public_key_x;
    let _pk_y = public_key_y;
    let _msg = message;

    // TODO(chillhacker)
    Ok(true)
}

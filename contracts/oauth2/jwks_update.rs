extern crate alloc;

use crate::{errors::OAuth2Error, jwks::JWKS};
use fluentbase_sdk::{ContextReader, SharedAPI};

/// Update JWKS for a provider
///
/// This function validates and stores new JWKS on-chain, allowing
/// key updates without contract redeployment
///
/// NOTE: Temporarily disabled due to SDK storage API changes
/// TODO(chillhacker): Re-enable when new storage API is finalized
///
#[allow(dead_code)]
fn update_jwks_disabled(
    sdk: &impl SharedAPI,
    provider: &str,
    jwks_json: &str,
) -> Result<(), OAuth2Error> {
    // Validate JWKS format
    let jwks: JWKS = serde_json::from_str(jwks_json).map_err(|_| OAuth2Error::InvalidJson)?;

    // Validate JWKS has at least one key
    if jwks.keys.is_empty() {
        return Err(OAuth2Error::JwksNotFound);
    }

    // Validate each key has required fields
    for key in &jwks.keys {
        if key.kty.is_empty() {
            return Err(OAuth2Error::KeyNotFound);
        }

        // RSA keys must have n and e
        if key.kty == "RSA" {
            if key.n.is_none() || key.e.is_none() {
                return Err(OAuth2Error::KeyNotFound);
            }
        }

        // EC keys must have x, y, and crv
        if key.kty == "EC" {
            if key.x.is_none() || key.y.is_none() || key.crv.is_none() {
                return Err(OAuth2Error::KeyNotFound);
            }
        }
    }

    // Store JWKS on-chain
    let storage_key = alloc::format!("jwks:{}", provider);
    sdk.storage_write(storage_key.as_bytes(), jwks_json.as_bytes());

    // Store update timestamp
    let time_key = alloc::format!("jwks:{}:updated_at", provider);
    let current_time = sdk.context().block_timestamp();
    sdk.storage_write(time_key.as_bytes(), &current_time.to_le_bytes());

    // Store key IDs for easy lookup
    let mut kids = alloc::vec![];
    for key in &jwks.keys {
        if let Some(kid) = &key.kid {
            kids.push(kid.clone());
        }
    }
    let kids_json = serde_json::to_string(&kids).unwrap_or_default();
    let kids_key = alloc::format!("jwks:{}:kids", provider);
    sdk.storage_write(kids_key.as_bytes(), kids_json.as_bytes());

    Ok(())
}

/// Get JWKS for a provider with storage fallback
///
/// This function:
/// 1. Checks on-chain storage for updated JWKS (if available)
/// 2. Falls back to hardcoded JWKS if not found
/// 3. Returns the most recent JWKS
pub fn get_jwks_with_fallback(sdk: &impl SharedAPI, provider: &str) -> Result<JWKS, OAuth2Error> {
    // Try to load from storage first
    let storage_key = alloc::format!("jwks:{}", provider);

    if let Some(stored_data) = sdk.storage_read(storage_key.as_bytes()) {
        // Try to parse stored JWKS
        if let Ok(stored_str) = core::str::from_utf8(&stored_data) {
            if let Ok(jwks) = serde_json::from_str::<JWKS>(stored_str) {
                return Ok(jwks);
            }
        }
    }

    // Fall back to hardcoded JWKS
    let jwks = match provider {
        "google" => crate::jwks_data::google_jwks(),
        _ => return Err(OAuth2Error::JwksNotFound),
    };

    Ok(jwks)
}

/// Get JWKS update timestamp
pub fn get_jwks_update_time(sdk: &impl SharedAPI, provider: &str) -> Option<u64> {
    let time_key = alloc::format!("jwks:{}:updated_at", provider);

    if let Some(time_bytes) = sdk.storage_read(time_key.as_bytes()) {
        if time_bytes.len() >= 8 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&time_bytes[..8]);
            return Some(u64::from_le_bytes(bytes));
        }
    }

    None
}

/// Check if JWKS is stale (older than threshold)
pub fn is_jwks_stale(sdk: &impl SharedAPI, provider: &str, max_age_seconds: u64) -> bool {
    if let Some(updated_at) = get_jwks_update_time(sdk, provider) {
        let current_time = sdk.context().block_timestamp();
        current_time - updated_at > max_age_seconds
    } else {
        // TODO(chillhacker): Implement on-chain JWKS update mechanism
        true
    }
}

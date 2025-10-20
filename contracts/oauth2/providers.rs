extern crate alloc;

use crate::config;
use crate::errors::OAuth2Error;
use crate::jwks::JWKS;
use crate::jwks_data;
use alloc::string::String;

/// OAuth2 Provider configuration
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub issuer: String,
    pub jwks: JWKS,
}

/// Get provider configuration by issuer (using config module)
pub fn get_provider_by_issuer(issuer: &str) -> Result<ProviderConfig, OAuth2Error> {
    // Try to get config from config module
    let provider_config =
        config::get_provider_config_by_issuer(issuer).ok_or(OAuth2Error::InvalidIssuer)?;

    // Map config to legacy ProviderConfig with JWKS
    // Use hardcoded JWKS (will be enhanced to check storage in production)
    let jwks = match provider_config.name.as_str() {
        "google" => jwks_data::google_jwks(),
        _ => JWKS {
            keys: alloc::vec![],
        },
    };

    Ok(ProviderConfig {
        name: provider_config.name,
        issuer: provider_config.issuer,
        jwks,
    })
}

/// Get provider configuration with storage fallback (for production use)
///
/// This function checks on-chain storage for updated JWKS first,
/// then falls back to hardcoded keys if not found
///
/// NOTE: Temporarily disabled due to SDK storage API changes
/// TODO: Re-enable when new storage API is finalized
#[allow(dead_code)]
fn get_provider_by_issuer_with_storage_disabled(
    _sdk: &impl fluentbase_sdk::SharedAPI,
    issuer: &str,
) -> Result<ProviderConfig, OAuth2Error> {
    // For now, just use hardcoded JWKS
    // Storage functionality will be added back when new API is ready
    get_provider_by_issuer(issuer)
}

/// Helper to get hardcoded JWKS
fn get_hardcoded_jwks(provider: &str) -> JWKS {
    match provider {
        "google" => jwks_data::google_jwks(),
        _ => JWKS {
            keys: alloc::vec![],
        },
    }
}

// Note: SharedAPI already provides storage_read/storage_write methods

/// Get provider configuration by name (using config module)
pub fn get_provider_by_name(name: &str) -> Result<ProviderConfig, OAuth2Error> {
    let provider_config = config::get_provider_config(name).ok_or(OAuth2Error::InvalidIssuer)?;

    let jwks = match provider_config.name.as_str() {
        "google" => jwks_data::google_jwks(),
        _ => JWKS {
            keys: alloc::vec![],
        },
    };

    Ok(ProviderConfig {
        name: provider_config.name,
        issuer: provider_config.issuer,
        jwks,
    })
}

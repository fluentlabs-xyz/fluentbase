extern crate alloc;

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// OAuth2 Provider Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider name (e.g., "google", "github")
    pub name: String,
    /// Issuer URL
    pub issuer: String,
    /// JWKS endpoint URL (if applicable)
    pub jwks_endpoint: Option<String>,
    /// Supported algorithms
    pub algorithms: Vec<String>,
    /// Whether this provider uses JWT tokens
    pub uses_jwt: bool,
    /// API verification endpoint (for non-JWT providers like GitHub)
    pub api_verify_endpoint: Option<String>,
    /// Token type (e.g., "Bearer", "token")
    pub token_type: String,
    /// Additional configuration
    pub extra: ProviderExtra,
}

/// Provider-specific extra configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderExtra {
    /// JWKS cache TTL in seconds
    pub jwks_cache_ttl: u64,
    /// Whether to verify email claim
    pub require_email_verified: bool,
    /// Custom claim mappings
    pub claim_mapping: ClaimMapping,
}

/// Claim field mappings (different providers use different field names)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimMapping {
    pub user_id: String, // Default: "sub"
    pub email: String,   // Default: "email"
    pub name: String,    // Default: "name"
    pub picture: String, // Default: "picture"
}

impl Default for ClaimMapping {
    fn default() -> Self {
        Self {
            user_id: "sub".into(),
            email: "email".into(),
            name: "name".into(),
            picture: "picture".into(),
        }
    }
}

impl Default for ProviderExtra {
    fn default() -> Self {
        Self {
            jwks_cache_ttl: 3600, // 1 hour
            require_email_verified: false,
            claim_mapping: ClaimMapping::default(),
        }
    }
}

/// Get all provider configurations
pub fn get_all_providers() -> Vec<ProviderConfig> {
    alloc::vec![
        google_config(),
        github_config(),
        discord_config(),
        apple_config(),
        microsoft_config(),
    ]
}

/// Google OAuth2/OIDC Configuration
pub fn google_config() -> ProviderConfig {
    ProviderConfig {
        name: "google".into(),
        issuer: "https://accounts.google.com".into(),
        jwks_endpoint: Some("https://www.googleapis.com/oauth2/v3/certs".into()),
        algorithms: alloc::vec!["RS256".into()],
        uses_jwt: true,
        api_verify_endpoint: None,
        token_type: "Bearer".into(),
        extra: ProviderExtra {
            jwks_cache_ttl: 3600,
            require_email_verified: true,
            claim_mapping: ClaimMapping::default(),
        },
    }
}

/// GitHub OAuth2 Configuration
pub fn github_config() -> ProviderConfig {
    ProviderConfig {
        name: "github".into(),
        issuer: "https://github.com".into(),
        jwks_endpoint: None,       // GitHub doesn't use JWKS
        algorithms: alloc::vec![], // API-based verification
        uses_jwt: false,
        api_verify_endpoint: Some("https://api.github.com/user".into()),
        token_type: "token".into(),
        extra: ProviderExtra {
            jwks_cache_ttl: 0,
            require_email_verified: false,
            claim_mapping: ClaimMapping {
                user_id: "id".into(),
                email: "email".into(),
                name: "login".into(),
                picture: "avatar_url".into(),
            },
        },
    }
}

/// Discord OAuth2 Configuration
pub fn discord_config() -> ProviderConfig {
    ProviderConfig {
        name: "discord".into(),
        issuer: "https://discord.com".into(),
        jwks_endpoint: None, // Check Discord docs for JWKS endpoint
        algorithms: alloc::vec!["RS256".into()],
        uses_jwt: true, // Assuming JWT, verify with Discord docs
        api_verify_endpoint: Some("https://discord.com/api/users/@me".into()),
        token_type: "Bearer".into(),
        extra: ProviderExtra {
            jwks_cache_ttl: 3600,
            require_email_verified: false,
            claim_mapping: ClaimMapping {
                user_id: "id".into(),
                email: "email".into(),
                name: "username".into(),
                picture: "avatar".into(),
            },
        },
    }
}

/// Apple Sign In Configuration
pub fn apple_config() -> ProviderConfig {
    ProviderConfig {
        name: "apple".into(),
        issuer: "https://appleid.apple.com".into(),
        jwks_endpoint: Some("https://appleid.apple.com/auth/keys".into()),
        algorithms: alloc::vec!["RS256".into()],
        uses_jwt: true,
        api_verify_endpoint: None,
        token_type: "Bearer".into(),
        extra: ProviderExtra {
            jwks_cache_ttl: 86400, // 24 hours (Apple keys rotate less frequently)
            require_email_verified: true,
            claim_mapping: ClaimMapping::default(),
        },
    }
}

/// Microsoft/Azure AD Configuration
pub fn microsoft_config() -> ProviderConfig {
    ProviderConfig {
        name: "microsoft".into(),
        issuer: "https://login.microsoftonline.com".into(),
        jwks_endpoint: Some("https://login.microsoftonline.com/common/discovery/v2.0/keys".into()),
        algorithms: alloc::vec!["RS256".into()],
        uses_jwt: true,
        api_verify_endpoint: None,
        token_type: "Bearer".into(),
        extra: ProviderExtra {
            jwks_cache_ttl: 3600,
            require_email_verified: false,
            claim_mapping: ClaimMapping {
                user_id: "oid".into(), // Microsoft uses "oid" for user ID
                email: "email".into(),
                name: "name".into(),
                picture: "picture".into(),
            },
        },
    }
}

/// Get provider config by name
pub fn get_provider_config(name: &str) -> Option<ProviderConfig> {
    match name {
        "google" => Some(google_config()),
        "github" => Some(github_config()),
        "discord" => Some(discord_config()),
        "apple" => Some(apple_config()),
        "microsoft" => Some(microsoft_config()),
        _ => None,
    }
}

/// Get provider config by issuer
pub fn get_provider_config_by_issuer(issuer: &str) -> Option<ProviderConfig> {
    get_all_providers()
        .into_iter()
        .find(|p| p.issuer == issuer || issuer.starts_with(&p.issuer))
}

/// Load configuration from JSON (optional feature)
#[cfg(feature = "json-config")]
pub fn load_config_from_json(json: &str) -> Result<Vec<ProviderConfig>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Export configuration to JSON (for documentation/debugging)
#[cfg(feature = "json-config")]
pub fn export_config_to_json(pretty: bool) -> Result<String, serde_json::Error> {
    let configs = get_all_providers();
    if pretty {
        serde_json::to_string_pretty(&configs)
    } else {
        serde_json::to_string(&configs)
    }
}

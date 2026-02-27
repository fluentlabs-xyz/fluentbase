#![allow(clippy::unnecessary_mut_passed)]
//! Universal Token SDK
//!
//! This module provides utilities for deploying and interacting with Universal Token contracts.
//! The Universal Token is a precompile-based ERC20 implementation that uses a shared runtime.
//!
//! # How It Works
//!
//! When you deploy a Universal Token:
//!
//! 1. You send a CREATE transaction with constructor data = `MAGIC_BYTES + encoded(InitialSettings)`
//! 2. The system detects the magic bytes and routes to `PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME`
//! 3. The runtime's `deploy_entry` function initializes the token at the created address
//! 4. Each subsequent call to that address is routed to the same runtime
//!
//! This is **not** a factory because:
//! - No factory contract exists
//! - Each token is deployed independently via CREATE
//! - The runtime is shared, but storage is per-address
//! - No `createToken()` function - you just deploy with the magic bytes

extern crate alloc;

use crate::{
    bytes::BytesMut,
    codec::{Codec, SolidityABI},
    Address, Bytes, U256,
};
use alloc::{string::String, vec::Vec};

/// Re-export the precompile address for convenience
pub use fluentbase_types::PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME;

/// Re-export the magic bytes constant
pub use fluentbase_types::UNIVERSAL_TOKEN_MAGIC_BYTES;

/// Internal structure matching `TokenNameOrSymbol` from universal-token
/// This is a 32-byte array used for token name and symbol storage
#[derive(Default, Debug, Clone, PartialEq, Eq, Codec)]
#[repr(transparent)]
struct TokenNameOrSymbol {
    bytes: [u8; U256::BYTES],
}

impl TokenNameOrSymbol {
    fn from_str(value: &str) -> Self {
        let mut bytes = [0u8; U256::BYTES];
        let len = core::cmp::min(U256::BYTES, value.len());
        bytes[..len].copy_from_slice(value.as_bytes());
        Self { bytes }
    }
}

/// Internal structure matching `InitialSettings` from universal-token
/// This matches the encoding format expected by the universal token runtime
#[derive(Debug, Clone, Codec)]
struct InitialSettings {
    token_name: TokenNameOrSymbol,
    token_symbol: TokenNameOrSymbol,
    decimals: u8,
    initial_supply: U256,
    minter: Address,
    pauser: Address,
}

/// Builder for Universal Token deployment configuration
#[derive(Debug, Clone)]
pub struct TokenConfig {
    /// Token name (e.g., "My Token")
    pub name: String,
    /// Token symbol (e.g., "MTK")
    pub symbol: String,
    /// Number of decimals (typically 18)
    pub decimals: u8,
    /// Initial supply to mint to the deployer
    pub initial_supply: U256,
    /// Optional minter address (enables minting functionality)
    pub minter: Option<Address>,
    /// Optional pauser address (enables pause/unpause functionality)
    pub pauser: Option<Address>,
}

impl TokenConfig {
    /// Create a new token configuration builder
    pub fn builder() -> TokenConfigBuilder {
        TokenConfigBuilder::new()
    }

    /// Create a deployment transaction payload
    ///
    /// This returns the constructor data that should be used in a CREATE transaction.
    /// The data consists of:
    /// 1. `UNIVERSAL_TOKEN_MAGIC_BYTES` (4 bytes) - identifies this as a Universal Token
    /// 2. Encoded `InitialSettings` - the token configuration
    ///
    /// # Returns
    ///
    /// The constructor data as `Bytes` that should be sent in a CREATE transaction.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = TokenConfig::builder()
    ///     .name("My Token")
    ///     .symbol("MTK")
    ///     .decimals(18)
    ///     .initial_supply(1_000_000_000_000_000_000_000_000u128)
    ///     .build();
    ///
    /// ```
    pub fn create_deployment_transaction(&self) -> Bytes {
        // Create the InitialSettings structure compatible with universal-token
        let settings = InitialSettings {
            token_name: TokenNameOrSymbol::from_str(&self.name),
            token_symbol: TokenNameOrSymbol::from_str(&self.symbol),
            decimals: self.decimals,
            initial_supply: self.initial_supply,
            minter: self.minter.unwrap_or(Address::ZERO),
            pauser: self.pauser.unwrap_or(Address::ZERO),
        };

        // Encode using Solidity ABI
        let mut bytes = BytesMut::new();
        SolidityABI::encode(&settings, &mut bytes, 0).unwrap();
        let encoded = bytes.freeze();

        // Prepend magic bytes
        let mut output = Vec::with_capacity(UNIVERSAL_TOKEN_MAGIC_BYTES.len() + encoded.len());
        output.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
        output.extend_from_slice(encoded.as_ref());
        output.into()
    }
}

/// Builder for `TokenConfig`
#[derive(Debug, Clone, Default)]
pub struct TokenConfigBuilder {
    name: Option<String>,
    symbol: Option<String>,
    decimals: Option<u8>,
    initial_supply: Option<U256>,
    minter: Option<Address>,
    pauser: Option<Address>,
}

impl TokenConfigBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the token name
    pub fn name<S: Into<String>>(mut self, name: S) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the token symbol
    pub fn symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// Set the number of decimals (default: 18)
    pub fn decimals(mut self, decimals: u8) -> Self {
        self.decimals = Some(decimals);
        self
    }

    /// Set the initial supply (default: 0)
    pub fn initial_supply<U>(mut self, supply: U) -> Self
    where
        U: Into<U256>,
    {
        self.initial_supply = Some(supply.into());
        self
    }

    /// Set the minter address (enables minting)
    pub fn minter(mut self, minter: Address) -> Self {
        self.minter = Some(minter);
        self
    }

    /// Set the pauser address (enables pause/unpause)
    pub fn pauser(mut self, pauser: Address) -> Self {
        self.pauser = Some(pauser);
        self
    }

    /// Build the `TokenConfig`
    ///
    /// # Panics
    ///
    /// Panics if required fields (name, symbol) are not set.
    pub fn build(self) -> TokenConfig {
        TokenConfig {
            name: self.name.expect("token name is required"),
            symbol: self.symbol.expect("token symbol is required"),
            decimals: self.decimals.unwrap_or(18),
            initial_supply: self.initial_supply.unwrap_or(U256::ZERO),
            minter: self.minter,
            pauser: self.pauser,
        }
    }

    /// Build the `TokenConfig` or return an error
    pub fn try_build(self) -> Result<TokenConfig, TokenConfigError> {
        Ok(TokenConfig {
            name: self.name.ok_or(TokenConfigError::MissingName)?,
            symbol: self.symbol.ok_or(TokenConfigError::MissingSymbol)?,
            decimals: self.decimals.unwrap_or(18),
            initial_supply: self.initial_supply.unwrap_or(U256::ZERO),
            minter: self.minter,
            pauser: self.pauser,
        })
    }
}

/// Errors that can occur when building a token configuration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenConfigError {
    /// Token name was not provided
    MissingName,
    /// Token symbol was not provided
    MissingSymbol,
}

impl core::fmt::Display for TokenConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TokenConfigError::MissingName => write!(f, "token name is required"),
            TokenConfigError::MissingSymbol => write!(f, "token symbol is required"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TokenConfigError {}

/// Helper function to create a deployment transaction
///
/// This is a convenience function that creates a token configuration and
/// returns the deployment transaction data in one call.
///
/// # Arguments
///
/// * `name` - Token name
/// * `symbol` - Token symbol
/// * `decimals` - Number of decimals (default: 18)
/// * `initial_supply` - Initial supply to mint (default: 0)
///
/// # Example
///
/// ```rust,ignore
/// use fluentbase_sdk::universal_token::create_deployment_tx;
///
/// let constructor_data = create_deployment_tx(
///     "My Token",
///     "MTK",
///     18,
///     1_000_000_000_000_000_000_000_000u128,
/// );
/// ```
pub fn create_deployment_tx<U>(
    name: impl Into<String>,
    symbol: impl Into<String>,
    decimals: u8,
    initial_supply: U,
) -> Bytes
where
    U: Into<U256>,
{
    TokenConfig::builder()
        .name(name)
        .symbol(symbol)
        .decimals(decimals)
        .initial_supply(initial_supply)
        .build()
        .create_deployment_transaction()
}

/// Helper function to create a deployment transaction with roles
///
/// Similar to `create_deployment_tx` but allows setting minter and pauser addresses.
pub fn create_deployment_tx_with_roles<U>(
    name: impl Into<String>,
    symbol: impl Into<String>,
    decimals: u8,
    initial_supply: U,
    minter: Option<Address>,
    pauser: Option<Address>,
) -> Bytes
where
    U: Into<U256>,
{
    let mut builder = TokenConfig::builder()
        .name(name)
        .symbol(symbol)
        .decimals(decimals)
        .initial_supply(initial_supply);

    if let Some(minter) = minter {
        builder = builder.minter(minter);
    }

    if let Some(pauser) = pauser {
        builder = builder.pauser(pauser);
    }

    builder.build().create_deployment_transaction()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_config_builder() {
        let config = TokenConfig::builder()
            .name("Test Token")
            .symbol("TST")
            .decimals(18)
            .initial_supply(U256::from_be_bytes({
                let mut bytes = [0u8; 32];
                bytes[16..].copy_from_slice(&1_000_000_000_000_000_000_000_000u128.to_be_bytes());
                bytes
            }))
            .build();

        assert_eq!(config.name, "Test Token");
        assert_eq!(config.symbol, "TST");
        assert_eq!(config.decimals, 18);
        // initial_supply should match the exact U256 value we constructed above.
        let expected = U256::from_be_bytes({
            let mut bytes = [0u8; 32];
            bytes[16..].copy_from_slice(&1_000_000_000_000_000_000_000_000u128.to_be_bytes());
            bytes
        });
        assert_eq!(config.initial_supply, expected);
    }

    #[test]
    fn test_token_config_defaults() {
        let config = TokenConfig::builder().name("Test").symbol("TST").build();

        assert_eq!(config.decimals, 18);
        assert_eq!(config.initial_supply, U256::ZERO);
        assert_eq!(config.minter, None);
        assert_eq!(config.pauser, None);
    }

    #[test]
    fn test_deployment_transaction_has_magic_bytes() {
        let config = TokenConfig::builder().name("Test").symbol("TST").build();

        let tx_data = config.create_deployment_transaction();
        let magic_bytes = &tx_data[..4];
        assert_eq!(magic_bytes, UNIVERSAL_TOKEN_MAGIC_BYTES);
    }

    #[test]
    fn test_create_deployment_tx_helper() {
        let tx_data = create_deployment_tx(
            "Test",
            "TST",
            18,
            U256::from_be_bytes({
                let mut bytes = [0u8; 32];
                bytes[24..].copy_from_slice(&1000u64.to_be_bytes());
                bytes
            }),
        );
        assert_eq!(&tx_data[..4], UNIVERSAL_TOKEN_MAGIC_BYTES);
    }

    #[test]
    fn test_create_deployment_tx_with_roles() {
        let minter = Address::with_last_byte(1);
        let pauser = Address::with_last_byte(2);

        let tx_data = create_deployment_tx_with_roles(
            "Test",
            "TST",
            18,
            U256::from_be_bytes({
                let mut bytes = [0u8; 32];
                bytes[24..].copy_from_slice(&1000u64.to_be_bytes());
                bytes
            }),
            Some(minter),
            Some(pauser),
        );

        assert_eq!(&tx_data[..4], UNIVERSAL_TOKEN_MAGIC_BYTES);

        // Verify the settings can be decoded
        use crate::codec::SolidityABI;
        let mut buf: &[u8] = &tx_data[4..];
        let settings = SolidityABI::<InitialSettings>::decode(&mut buf, 0).unwrap();
        assert_eq!(settings.minter, minter);
        assert_eq!(settings.pauser, pauser);
    }
}

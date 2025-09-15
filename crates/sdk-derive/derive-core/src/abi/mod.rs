//! ABI module provides functionality for working with Solidity ABI, focusing on function signatures
//! parsing and type conversions.
//!
//! # Core components
//!
//! * `SolType` - represents Solidity types, parses Rust types into their Solidity equivalents
//! * `FunctionABI` - represents Solidity function definitions
//! * `RustToSol` - converts Rust types to their Solidity equivalents using registry
//!
//! # Function ID Generation
//!
//! Module can generate function signatures and IDs from Rust code in two modes:
//!
//! 1. Registry mode (enabled via feature flag):
//!    - All structure definitions are loaded into registry at startup
//!    - When constructing `FunctionABI`, registry is used to resolve types
//!    - Provides complete type information for complex structures
//!
//! 2. Direct mode (default):
//!    - No registry preloading
//!    - Types are converted directly during `FunctionABI` construction
//!    - Suitable for simple cases without complex nested structures
//!
//! ```rust, ignore
//! use crate::abi::{FunctionABI, RustToSol};
//!
//! // Parse Rust function signature
//! let sig: syn::Signature = parse_quote! {
//!     fn transfer(amount: u64, recipient: String) -> String
//! };
//!
//! // Create registry (optional)
//! // check artifacts for structure definitions
//!
//!  let config = ArtifactsRegistryConfig::new("OUT_DIR").with_mirror(".artifacts");
//! let registry = ArtifactsRegistry::new(config)?;
//!
//!
//! // Convert to FunctionABI
//! let abi = FunctionABI::from_syn(&sig, registry)?;
//!
//! // Get function ID (first 4 bytes of keccak256 hash)
//! let function_id = abi.function_id();
//!
//! // Get canonical function signature (e.g. "transfer(uint256,string)")
//! let signature = abi.signature();
//! ```
//!
//! # Constraints
//!
//! - Types must implement `SolidityABI` derive macro
//! - Generic types are not supported
//! - Module path used for type identification in registry
//! - Only non-generic Rust types can be converted to Solidity types
//! - Complex types must be registered in the registry when using Registry mode
pub mod constructor;
pub mod contract;
pub mod error;
pub mod function;
pub mod parameter;
pub mod types;

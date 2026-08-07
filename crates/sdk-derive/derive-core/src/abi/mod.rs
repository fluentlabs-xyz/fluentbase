//! ABI module provides functionality for working with Solidity ABI, focusing on function signatures
//! parsing and type conversions.
//!
//! # Core components
//!
//! * `SolType` - represents Solidity types, parses Rust types into their Solidity equivalents
//! * `FunctionABI` - represents Solidity function definitions
//! * `StructRegistry` - the crate's `#[derive(Codec)]` structs, used to expand struct parameters
//!
//! # Function ID Generation
//!
//! A Rust type is converted to its Solidity equivalent by `rust_to_sol`, which only sees the type
//! path: any unknown type becomes a struct with no components. Such a parameter has no canonical
//! signature, so a `FunctionABI` must be resolved through a [`structs::StructResolver`] before its
//! signature or function ID is taken. The router macro and the build tooling both do this, which is
//! what keeps the compiled dispatch table and the published artifacts on the same selector.
//!
//! ```rust, ignore
//! // Parse a Rust function signature
//! let sig: syn::Signature = parse_quote! {
//!     fn transfer(params: TransferParams) -> bool
//! };
//!
//! // Structs of the crate being compiled, parsed on demand
//! let resolver = StructResolver::crate_sources();
//!
//! // Convert to FunctionABI, expanding `TransferParams` into its components
//! let abi = FunctionABI::from_signature_with(&sig, &resolver)?;
//!
//! // Canonical function signature, e.g. "transfer((address,uint256))"
//! let signature = abi.signature()?;
//!
//! // Function ID (first 4 bytes of the keccak256 hash of that signature)
//! let function_id = abi.function_id()?;
//! ```
//!
//! # Constraints
//!
//! - Struct parameters must be declared with `#[derive(Codec)]` in the contract's own crate, or
//!   have their selector pinned with `#[function_id("...")]`
//! - Generic types are not supported
//! - Module path is used for type identification, so a bare name matching several modules is
//!   rejected rather than resolved arbitrarily
pub mod constructor;
pub mod contract;
pub mod error;
pub mod function;
pub mod parameter;
pub mod structs;
pub mod types;

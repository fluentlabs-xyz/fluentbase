use crate::contract::impl_derive_contract;
use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::quote;

mod contract;

mod utils;

use derive_core::{client_core, router_core, solidity_abi_core, storage_core};

/// Generates a dispatch mechanism for contract functions with configurable encoding modes.
///
/// This macro implements the routing logic for the contract's main entry point, automatically
/// handling:
/// - Function selector matching
/// - Parameter encoding/decoding
/// - Result encoding
/// - Error handling
///
/// # Encoding Modes
///
/// Specify the encoding mode using the `mode` parameter:
/// - `"solidity"`: Big-endian, 32-byte aligned, Ethereum ABI compatible
/// - `"fluent"`: Little-endian, 4-byte aligned, optimized for compact representation
///
/// # Implementation Types
///
/// The router supports two implementation approaches:
///
/// 1. Struct Implementation - Only public functions are included in dispatch:
/// ```rust,ignore
/// #[router(mode = "solidity")]
/// impl<SDK: SharedAPI> Contract<SDK> {
///     pub fn external_method(&mut self, data: String) -> String {
///         // This function will be routed
///         data
///     }
///
///     fn internal_helper(&self) -> bool {
///         // This function will NOT be routed
///         true
///     }
/// }
/// ```
///
/// 2. Trait Implementation - All trait functions are included in dispatch:
/// ```rust,ignore
/// trait ContractAPI {
///     fn greeting(&self, msg: String) -> String;
/// }
///
/// #[router(mode = "solidity")]
/// impl<SDK: SharedAPI> ContractAPI for Contract<SDK> {
///     fn greeting(&self, msg: String) -> String {
///         msg
///     }
/// }
/// ```
///
/// # Special Functions
///
/// - `deploy()` - Automatically excluded from routing (used only during deployment)
/// - `fallback()` - Optional handler for unknown function selectors:
/// ```rust,ignore
/// pub fn fallback(&mut self) {
///     // Handle unknown selectors
/// }
/// ```
///
/// # Type Requirements
///
/// - All parameter and return types must implement `fluentbase_sdk::codec::Encoder`
/// - References (`&T`, `&mut T`) are not allowed in function parameters
/// - See documentation for complete type mapping details
///
/// # Generated Artifacts
///
/// The macro automatically generates in the output directory:
/// - `{ContractName}.abi.json` - ABI specification
/// - `I{ContractName}.sol` - Solidity interface
///
/// Configure artifact location using either:
/// - `FLUENTBASE_CONTRACT_ARTIFACTS_DIR` environment variable
/// - `build.rs` script
///
/// # Errors
///
/// Compile-time errors occur when:
/// - Invalid encoding mode is specified
/// - Types don't implement required traits
/// - Function parameter references are used
/// - Invalid function selectors are provided
///
/// Runtime panics occur when:
/// - Input data is too short (< 4 bytes)
/// - Unknown function selector (unless fallback is implemented)
/// - Parameter decoding fails
///
/// See the full documentation for complete details and examples.
#[proc_macro_error]
#[proc_macro_attribute]
pub fn router(args: TokenStream, input: TokenStream) -> TokenStream {
    let router_impl = match router_core(args.into(), input.into()) {
        Ok(expanded) => expanded,
        Err(err) => abort!(err.span(), "{}", err),
    };

    router_impl.into()
}

/// Generates a client-side wrapper for interacting with smart contracts.
///
/// This macro automatically generates a client struct with methods for encoding parameters,
/// making contract calls, and decoding results. The generated client handles all the
/// low-level details of contract interaction.
///
/// # Usage
///
/// ```rust,ignore
/// #[client(mode = "solidity")]
/// trait TokenAPI {
///     #[function_id("transfer(address,uint256)")]
///     fn transfer(&mut self, to: Address, amount: U256) -> bool;
///
///     #[function_id("balanceOf(address)")]
///     fn balance_of(&mut self, owner: Address) -> U256;
/// }
/// ```
///
/// The macro generates a `{TraitName}Client<SDK>` struct implementing all trait methods.
///
/// # Generated Features
///
/// For each trait method, the client generates:
/// - Parameter encoding methods (`encode_{method_name}`)
/// - Result decoding methods (`decode_{method_name}`)
/// - Contract call implementations with gas and value management
/// - Safety checks for transaction requirements
///
/// # Encoding Modes
///
/// Specify the encoding mode using the `mode` parameter:
/// - `"solidity"`: Big-endian, 32-byte aligned, Ethereum ABI compatible
/// - `"fluent"`: Little-endian, 4-byte aligned, optimized for compact representation
///
/// # Method Requirements
///
/// - All methods must have a `function_id` attribute
/// - Parameter and return types must implement required encoding traits
/// - Methods can't use references in parameters
///
/// # Generated Implementation
///
/// ```rust,ignore
/// pub struct TokenAPIClient<SDK> {
///     pub sdk: SDK
/// }
///
/// impl<SDK: SharedAPI> TokenAPIClient<SDK> {
///     pub fn transfer(
///         &mut self,
///         contract_address: Address,
///         value: U256,
///         gas_limit: u64,
///         to: Address,
///         amount: U256
///     ) -> bool { ... }
/// }
/// ```
///
/// # Errors
///
/// Compile-time errors occur when:
/// - Invalid encoding mode is specified
/// - Missing function_id attributes
/// - Types don't implement required traits
/// - Invalid function selectors
///
/// Runtime errors occur when:
/// - Insufficient funds for transaction
/// - Insufficient gas limit
/// - Contract call fails (non-zero exit code)
#[proc_macro_error]
#[proc_macro_attribute]
pub fn client(args: TokenStream, input: TokenStream) -> TokenStream {
    let client_impl = match client_core(args.into(), input.into()) {
        Ok(expanded) => expanded,
        Err(err) => abort!(err.span(), "{}", err),
    };

    client_impl.into()
}

/// Specifies a custom function selector for contract methods.
///
/// This attribute allows overriding the default function selector calculation
/// (first 4 bytes of keccak256 hash) with a custom value. The selector can be
/// specified in three formats:
///
/// # Formats
///
/// 1. Solidity-style signature (will be hashed):
/// ```rust, ignore
/// #[function_id("transferAll(address,uint256)")]
/// fn transfer_all(&self, to: Address, amount: U256)
/// ```
///
/// 2. Direct hex representation:
/// ```rust, ignore
/// #[function_id("0xa9059cbb")]
/// fn transfer_all(&self, to: Address, amount: U256)
/// ```
///
/// 3. Raw bytes:
/// ```rust, ignore
/// #[function_id([169, 5, 156, 187])]
/// fn transfer_all(&self, to: Address, amount: U256)
/// ```
///
/// # Signature Validation
///
/// By default, when using the signature format, the macro validates that the
/// provided signature matches the function's actual parameters. This can be
/// disabled with the `validate` parameter:
///
/// ```rust, ignore
/// #[function_id("transferAll(address,uint256)", validate(false))]
/// fn transfer_all(&self, to: Address, amount: U256)
/// ```
///
/// # Function Name Convention
///
/// When using signature format, function names are automatically converted to
/// camelCase before hashing. For example:
/// - `transfer_all` becomes `transferAll`
/// - `get_balance_of` becomes `getBalanceOf`
///
/// # Errors
///
/// Compile-time errors occur when:
/// - Invalid selector format is provided
/// - Signature validation fails (when enabled)
/// - Selector length is not 4 bytes
#[proc_macro_attribute]
pub fn function_id(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Generates Solidity-compatible ABI specifications for Rust structs.
///
/// This macro automatically generates and saves ABI specifications for struct types,
/// which are used by the router macro to generate Solidity interfaces and ABI files.
/// The generated ABI follows Ethereum's standard ABI encoding rules.
///
/// # Usage
///
/// ```rust, ignore
/// #[derive(Codec, Debug, Clone, SolidityABI)]
/// pub struct Point {
///     x: u64,
///     y: U256,
/// }
/// ```
///
/// # Generated Artifacts
///
/// The macro automatically generates in the output directory:
/// - `{StructName}.json` - ABI specification file in `$OUT_DIR/solidity_abi/`
///
/// The generated ABI file contains:
/// - Component types and names
/// - Nested struct definitions
/// - Type mappings from Rust to Solidity
///
/// # Type Requirements
///
/// - All struct fields must implement required encoding traits
/// - Unnamed tuple structs are not supported
/// - Nested structs must also derive `SolidityABI`
///
/// # Type Mappings
///
/// Basic mappings from Rust to Solidity types:
/// - `u64` → `uint64`
/// - `U256` → `uint256`
/// - `String` → `string`
/// - `bool` → `bool`
/// - Nested structs → `tuple`
///
/// # Errors
///
/// Compile-time errors occur when:
/// - Deriving for non-struct types
/// - Using unnamed fields
/// - Using unsupported field types
/// - Unable to create output directory
/// - Unable to write ABI file
///
/// See the router macro documentation for details on how these ABI files are used
/// in the contract interface generation process.
#[proc_macro_derive(SolidityABI)]
pub fn derive_solidity_abi(input: TokenStream) -> TokenStream {
    let input: proc_macro2::TokenStream = input.into();
    match solidity_abi_core(input) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Generates Solidity-compatible storage patterns for smart contract state variables.
///
/// This macro implements Solidity's storage layout patterns in Rust, automatically handling
/// slot management, key calculations, and access methods. It provides a type-safe way to
/// interact with contract storage while maintaining compatibility with Solidity's storage layout.
///
/// # Usage
///
/// ```rust, ignore
/// solidity_storage! {
///     // Simple values
///     Address Owner;           // Slot 0
///     U256 Balance;           // Slot 1
///
///     // Mappings
///     mapping(Address => U256) Balances;                          // Single mapping
///     mapping(Address => mapping(Address => U256)) Allowances;    // Nested mapping
///
///     // Arrays
///     U256[] Values;                 // Dynamic array
///     Address[][][] NestedArray;     // Multi-dimensional array
///
///     // Custom structures
///     UserData Data;                 // Custom struct
///     mapping(Address => UserData) UserMap;  // Struct in mapping
/// }
/// ```
///
/// # Storage Layout
///
/// The macro follows Solidity's storage layout rules:
/// - Sequential slot assignment starting from 0
/// - Mapping slots contain no value but are used for key calculation
/// - Dynamic array slots store length, with data at `keccak256(slot)`
/// - Structs are stored contiguously from their assigned slot
///
/// # Generated Features
///
/// For each storage variable, the macro generates:
/// - Slot constants and key calculation methods
/// - Getter methods: `get<SDK: SharedAPI>(sdk: &SDK, ...args) -> T`
/// - Setter methods: `set<SDK: SharedAPI>(sdk: &mut SDK, ...args, value: T)`
///
/// # Type Requirements
///
/// - Custom types must implement the `Codec` trait:
/// ```rust, ignore
/// #[derive(Codec)]
/// pub struct UserData {
///     balance: U256,
///     active: bool,
/// }
/// ```
///
/// # Type Support
///
/// - Basic types: `U256`, `Address`, `Bytes`, `bool`
/// - Complex types:
///   - `mapping(K => V)` - Single and nested mappings
///   - `T[]` - Single and multi-dimensional arrays
///   - Custom structs implementing `Codec`
///
/// # Errors
///
/// Compile-time errors occur when:
/// - Using unsupported types
/// - Invalid storage patterns
/// - Missing trait implementations
/// - Invalid slot calculations
///
/// Runtime errors occur when:
/// - Storage access fails
/// - Type conversion errors
/// - Invalid key calculations
#[proc_macro]
#[proc_macro_error]
pub fn solidity_storage(token: TokenStream) -> TokenStream {
    let storage_impl = match storage_core(token.into()) {
        Ok(expanded) => expanded,
        Err(err) => abort!(err.span(), "{}", err),
    };

    storage_impl.into()
}

#[proc_macro]
pub fn derive_keccak256_id(token: TokenStream) -> TokenStream {
    let signature = token.to_string();
    let method_id = utils::calculate_keccak256_id(&signature);
    TokenStream::from(quote! {
        #method_id
    })
}

#[proc_macro]
pub fn derive_keccak256(token: TokenStream) -> TokenStream {
    let signature = token.to_string();
    let method_id = utils::calculate_keccak256(&signature);
    TokenStream::from(quote! {
        [#(#method_id,)*]
    })
}

#[proc_macro_derive(Contract)]
pub fn contract_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_derive_contract(&ast)
}

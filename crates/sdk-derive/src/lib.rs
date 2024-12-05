use crate::contract::impl_derive_contract;
use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::quote;

mod contract;

mod utils;

use derive_core::{client_core, router_core, storage_core};

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
///
/// See the router macro documentation for complete details on function selectors.
#[proc_macro_attribute]
pub fn function_id(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

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

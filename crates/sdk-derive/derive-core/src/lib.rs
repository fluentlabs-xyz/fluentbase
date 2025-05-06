//! Core functionality for the router macro implementation.
//! This crate provides the base functionality used by the proc-macro crate.

use alloy_sol_macro_input::{SolInput, SolInputKind};
pub use fluentbase_codec::bytes::{Buf, BufMut, Bytes, BytesMut};
use mode::RouterMode;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use router::Router;
use storage::Storage;
use syn::LitStr;
use syn_solidity::{parse2 as sol_parse2, File, Item, ItemFunction, Spanned};
use tracing::{debug, error, info};

pub mod abi;
pub mod args;
pub mod artifacts;
pub mod client;
pub mod codec;
pub mod error;
pub mod expand_from_sol_input;
pub mod function_id;
pub mod mode;
pub mod route;
pub mod router;
pub mod storage;
pub mod utils;

/// Processes the router macro invocation.
///
/// # Arguments
/// * `attr` - Attribute TokenStream containing router configuration
/// * `input` - Input TokenStream containing the router implementation
///
/// # Returns
/// * `Result<TokenStream2, syn::Error>` - Processed router code or error
pub fn router_core(attr: TokenStream2, input: TokenStream2) -> Result<TokenStream2, syn::Error> {
    debug!("Processing router attributes");

    let args = parse_router_args(attr)?;
    info!("Initialized router with mode: {:?}", args.mode);

    let mut router = parse_router_input(input)?;
    router.mode = args.mode;
    router.artifacts_path = args.artifacts_path;
    if let Some(artifacts_path) = &router.artifacts_path {
        info!("Artifacts will be generated at: {}", artifacts_path);

        if router.mode == RouterMode::Solidity {
            router.generate_artifacts(artifacts_path);
        }
    }

    Ok(quote!(#router))
}

/// Parses router arguments from the attribute TokenStream.
fn parse_router_args(attr: TokenStream2) -> Result<args::RouterArgs, syn::Error> {
    debug!("Parsing router arguments");
    syn::parse2(attr).map_err(|e| {
        error!("Failed to parse router arguments: {}", e);
        e
    })
}

/// Parses router implementation from the input TokenStream.
fn parse_router_input(input: TokenStream2) -> Result<Router, syn::Error> {
    debug!("Parsing router implementation");
    syn::parse2(input)
}

/// Processes the client macro invocation.
///
/// # Arguments
/// * `attr` - Attribute TokenStream containing client configuration
/// * `input` - Input TokenStream containing the trait definition
///
/// # Returns
/// * `Result<TokenStream2, syn::Error>` - Processed client code or error
pub fn client_core(attr: TokenStream2, input: TokenStream2) -> Result<TokenStream2, syn::Error> {
    debug!("Processing client attributes");

    let args = parse_client_args(attr)?;
    info!("Initialized client with mode: {:?}", args.mode);

    let mut generator = parse_client_input(input)?;
    generator.args = args;

    Ok(quote!(#generator).into())
}

/// Parses client arguments from the attribute TokenStream.
fn parse_client_args(attr: TokenStream2) -> Result<args::RouterArgs, syn::Error> {
    debug!("Parsing client arguments");
    syn::parse2(attr).map_err(|e| {
        error!("Failed to parse client arguments: {}", e);
        e
    })
}

/// Parses trait definition from the input TokenStream.
fn parse_client_input(input: TokenStream2) -> Result<client::ClientGenerator, syn::Error> {
    debug!("Parsing trait definition");
    syn::parse2(input)
}

/// Processes the storage macro invocation.
///
/// # Arguments
/// * `attr` - Attribute TokenStream containing storage configuration
/// * `input` - Input TokenStream containing the storage implementation
///
/// # Returns
/// * `Result<TokenStream2, syn::Error>` - Processed storage code or error

pub fn storage_core(input: TokenStream2) -> Result<TokenStream2, syn::Error> {
    debug!("Processing storage attributes");

    let storage: Storage = parse_storage_input(input)?;

    Ok(quote!(#storage).into())
}

/// Parses storage implementation from the input TokenStream.
fn parse_storage_input(input: TokenStream2) -> Result<storage::Storage, syn::Error> {
    debug!("Parsing storage implementation");
    syn::parse2(input)
}

// pub fn expand_from_sol_input(input: SolInput) -> Result<TokenStream2, syn::Error> {
//     let file = match input.kind {
//         SolInputKind::Sol(sol_file) => sol_file,
//         SolInputKind::Type(_) => {
//             return Err(syn::Error::new(
//                 proc_macro2::Span::call_site(),
//                 "expected solidity file or interface",
//             ))
//         }
//         #[cfg(feature = "json")]
//         SolInputKind::Json(_, _) => {
//             return Err(syn::Error::new(
//                 proc_macro2::Span::call_site(),
//                 "JSON ABI not supported in this macro",
//             ))
//         }
//     };

//     let mut functions = vec![];

//     for item in file.items {
//         if let Item::Contract(interface) = item {
//             for i in &interface.body {
//                 if let Item::Function(ItemFunction {
//                     name: Some(name), ..
//                 }) = i
//                 {
//                     let fn_name = format_ident!("{}", name);
//                     functions.push(quote! {
//                         fn #fn_name(&self) {
//                             println!("called {}", stringify!(#fn_name));
//                         }
//                     });
//                 }
//             }
//         }
//     }

//     Ok(quote! {
//         pub trait DebugGenerated {
//             #(#functions)*
//         }
//     })
// }

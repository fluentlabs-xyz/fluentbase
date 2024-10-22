//! Core functionality for the router macro implementation.
//! This crate provides the base functionality used by the proc-macro crate.

pub use bytes::{Buf, BufMut, Bytes, BytesMut};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use router::Router;
use tracing::{debug, error, info};

pub mod args;
pub mod codec;
pub mod error;
pub mod function_id;
pub mod mode;
pub mod route;
pub mod router;
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

    Ok(quote!(#router).into())
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

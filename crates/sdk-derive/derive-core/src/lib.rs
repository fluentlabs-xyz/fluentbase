//! Core functionality for the router macro implementation.
//! This crate provides the base functionality used by the proc-macro crate.

pub use fluentbase_codec::bytes::{Buf, BufMut, Bytes, BytesMut};
use mode::RouterMode;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use router::Router;
use storage::Storage;
use tracing::{debug, error, info};

pub mod args;
pub mod client;
pub mod codec;
pub mod error;
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

    let mode = parse_router_args(attr)?;
    info!("Initialized router with mode: {:?}", mode);

    let mut router = parse_router_input(input)?;
    router.mode = mode;

    Ok(quote!(#router).into())
}

/// Parses router arguments from the attribute TokenStream.
fn parse_router_args(attr: TokenStream2) -> Result<RouterMode, syn::Error> {
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

    let mode = parse_client_args(attr)?;
    info!("Initialized client with mode: {:?}", mode);

    let mut generator = parse_client_input(input)?;
    generator.mode = mode;

    Ok(quote!(#generator).into())
}

/// Parses client arguments from the attribute TokenStream.
fn parse_client_args(attr: TokenStream2) -> Result<RouterMode, syn::Error> {
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

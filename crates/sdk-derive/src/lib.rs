use crate::contract::impl_derive_contract;
use fluentbase_sdk_derive_core::{client, router, storage};
use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use quote::{quote, ToTokens};

mod contract;
mod utils;
use syn::parse_macro_input;

/// Internal attribute used by the router macro.
/// This is not meant to be used directly by users.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn function_id(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[doc = include_str!("../docs/router.md")]
#[proc_macro_attribute]
#[proc_macro_error]
pub fn router(attr: TokenStream, input: TokenStream) -> TokenStream {
    match router::process_router(attr.into(), input.into()) {
        Ok(router) => router.to_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[doc = include_str!("../docs/client.md")]
#[proc_macro_attribute]
#[proc_macro_error]
pub fn client(attr: TokenStream, input: TokenStream) -> TokenStream {
    let attr_ts = proc_macro2::TokenStream::from(attr);
    let input_items = parse_macro_input!(input as syn::ItemTrait);

    match client::process_client(attr_ts, input_items.to_token_stream()) {
        Ok(client) => client.to_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[doc = include_str!("../docs/storage.md")]
#[proc_macro]
#[proc_macro_error]
pub fn solidity_storage(input: TokenStream) -> TokenStream {
    let storage = parse_macro_input!(input as storage::Storage);
    storage.to_token_stream().into()
}

#[doc = include_str!("../docs/solidity_trait.md")]
#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_trait(input: TokenStream) -> TokenStream {
    let parsed = syn::parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    fluentbase_sdk_derive_core::sol_input::to_rust_trait(parsed)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc = include_str!("../docs/solidity_client.md")]
#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_client(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    fluentbase_sdk_derive_core::sol_input::to_sol_client(parsed)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
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

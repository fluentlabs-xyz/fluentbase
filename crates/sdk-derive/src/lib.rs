use crate::contract::impl_derive_contract;
use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::quote;

mod contract;

mod utils;

use fluentbase_sdk_derive_core::{client_core, router_core, storage_core};
use syn::parse_macro_input;

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

#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_trait(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    fluentbase_sdk_derive_core::expand_from_sol_input::expand_from_sol_input(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
#[proc_macro_error]
pub fn derive_solidity_client(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as alloy_sol_macro_input::SolInput);

    match fluentbase_sdk_derive_core::expand_from_sol_input::expand_from_sol_input(parsed) {
        Ok(tokens) => {
            let code = tokens.to_string();
            let updated =
                code.replacen("pub trait", "#[client(mode = \"solidity\")]\npub trait", 1);
            updated.parse().unwrap_or_else(|err| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Parsing error: {err}"),
                )
                .to_compile_error()
                .into()
            })
        }
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_derive(Contract)]
pub fn contract_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_derive_contract(&ast)
}

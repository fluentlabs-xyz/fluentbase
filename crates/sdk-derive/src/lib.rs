use crate::contract::impl_derive_contract;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::{abort, proc_macro_error};
use quote::{quote, ToTokens};

mod contract;
mod solidity_storage;

mod utils;

use router_core::router_core;

#[proc_macro_error]
#[proc_macro_attribute]
pub fn router(args: TokenStream, input: TokenStream) -> TokenStream {
    let router_impl = match router_core(args.into(), input.into()) {
        Ok(expanded) => expanded,
        Err(err) => abort!(err.span(), "{}", err),
    };

    let result = quote! {
        #[allow(unused_imports)]
        use fluentbase_sdk::derive::function_id;

        #router_impl
    };

    result.into()
}

#[proc_macro_attribute]
pub fn function_id(attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro]
#[proc_macro_error]
pub fn solidity_storage(token: TokenStream) -> TokenStream {
    solidity_storage::SolidityStorage::expand(token)
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

// #[proc_macro_attribute]
// pub fn client(attr: TokenStream, item: TokenStream) -> TokenStream {
//     let args = parse_macro_input!(attr as RouterArgs);

//     let expanded = match args.mode {
//         RouterMode::Solidity => solidity_router::derive_solidity_client(
//             TokenStream::new(),
//             parse_macro_input!(item as ItemTrait),
//         )
//         .into(),
//         RouterMode::Codec => codec_router::derive_codec_client(
//             TokenStream::new(),
//             parse_macro_input!(item as ItemTrait),
//         ),
//     };
//     TokenStream::from(expanded)
// }

#[proc_macro_derive(Contract)]
pub fn contract_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_derive_contract(&ast)
}

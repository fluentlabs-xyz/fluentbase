use proc_macro::TokenStream;
use quote::quote;
use syn::{self, GenericParam};

pub(crate) fn impl_derive_contract(ast: &syn::DeriveInput) -> TokenStream {
    let struct_name = &ast.ident;
    for param in ast.generics.params.iter() {
        match param {
            GenericParam::Lifetime(_) => {}
            GenericParam::Type(_) => {}
            GenericParam::Const(_) => {}
        }
    }
    let output = quote! {
        impl Default for #struct_name<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager> {
            fn default() -> Self {
                #struct_name {
                    cr: &fluentbase_sdk::GuestContextReader::DEFAULT,
                    am: &fluentbase_sdk::GuestAccountManager::DEFAULT,
                }
            }
        }
    };
    TokenStream::from(output)
}

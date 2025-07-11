use proc_macro::TokenStream;
use quote::quote;
use syn::GenericParam;

pub(crate) fn impl_derive_contract(ast: &syn::DeriveInput) -> TokenStream {
    let struct_name = &ast.ident;
    ast.generics
        .params
        .iter()
        .find(|param| match param {
            GenericParam::Lifetime(_) => false,
            GenericParam::Type(val) => val.ident == "SDK",
            GenericParam::Const(_) => false,
        })
        .unwrap_or_else(|| panic!("missing SDK generic inside struct: {}", struct_name));
    let (impl_generics, type_generics, _where_clause) = ast.generics.split_for_impl();
    let output = quote! {
        impl #impl_generics #struct_name #type_generics {
            pub fn new(sdk: SDK) -> Self {
                #struct_name { sdk }
            }
        }
    };
    TokenStream::from(output)
}

use proc_macro::TokenStream;
use syn::{self, Data, Fields};

fn impl_derive_codec(ast: &syn::DeriveInput) -> TokenStream {
    let data_struct = match &ast.data {
        Data::Struct(data_struct) => data_struct,
        _ => panic!("not supported yet"),
    };
    let named_fields = match &data_struct.fields {
        Fields::Named(named_fields) => named_fields,
        _ => panic!("not supported yet"),
    };
    for field in named_fields.named.iter() {
        // field.ty;
    }
    panic!("not supported yet")
}

#[proc_macro_derive(Codec)]
pub fn codec_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_derive_codec(&ast)
}

#[cfg(test)]
mod tests {
    use crate::impl_derive_codec;
    use quote::quote;

    #[test]
    #[ignore]
    fn test_simple_struct() {
        let input = quote! {
            struct Test {
                a: u32,
                b: u32,
                c: u32,
            }
        };
        let ast = syn::parse(input.into()).unwrap();
        impl_derive_codec(&ast);
    }
}

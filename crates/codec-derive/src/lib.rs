#![allow(unused_imports)]
use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{__private::Span, format_ident, quote, ToTokens};
use syn::{self, Data, Fields, Ident};

#[proc_macro]
pub fn derive_keccak256_id(token: TokenStream) -> TokenStream {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(token.to_string());
    let mut dst = [0u8; 4];
    dst.copy_from_slice(hash.finalize().as_slice()[0..4].as_ref());
    let method_id: u32 = u32::from_be_bytes(dst);
    TokenStream::from(quote! {
        #method_id
    })
}

#[proc_macro]
pub fn path_to_test_name(token: TokenStream) -> TokenStream {
    let path = token.to_string();
    let file_name = path
        .split("/")
        .last()
        .expect("there is no last part in the path");
    let file_name = file_name.replace(".", "_").replace("\"", "");
    let file_ident = Ident::new_raw(file_name.as_str(), Span::call_site());
    TokenStream::from(quote! {
        #file_ident
    })
}

fn impl_derive_codec(ast: &syn::DeriveInput) -> TokenStream {
    let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let crate_name = if crate_name == "fluentbase-codec" {
        quote! { crate }
    } else {
        quote! { fluentbase_codec }
    };
    let data_struct = match &ast.data {
        Data::Struct(data_struct) => data_struct,
        _ => panic!("only structs are supported"),
    };
    let named_fields = match &data_struct.fields {
        Fields::Named(named_fields) => named_fields,
        _ => panic!("only named fields are supported"),
    };
    let header_sizes = named_fields.named.iter().map(|field| {
        let ty = &field.ty;
        quote! {
            <#ty as #crate_name::Encoder<#ty>>::HEADER_SIZE
        }
    });
    let encode_types = named_fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().unwrap();
        let ty = &field.ty;
        quote! {
            self.#ident.encode(encoder, field_offset);
            field_offset += <#ty as #crate_name::Encoder<#ty>>::HEADER_SIZE;
        }
    });
    let decode_types = named_fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().unwrap();
        let ty = &field.ty;
        quote! {
            <#ty as #crate_name::Encoder<#ty>>::decode_body(decoder, field_offset, &mut result.#ident);
            field_offset += <#ty as #crate_name::Encoder<#ty>>::HEADER_SIZE;
        }
    });
    let impl_types = named_fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().unwrap();
        let span = ident.span();
        let ident = ident.to_string().to_case(Case::Pascal);
        let ident = Ident::new(ident.as_str(), span);
        quote! {
            type #ident;
        }
    });
    let field_names = named_fields.named.clone();
    let impl_defs = named_fields.named.iter().enumerate().map(|(i, field)| {
        let ident = field.ident.as_ref().unwrap();
        let span = ident.span();
        let ident = ident.to_string().to_case(Case::Pascal);
        let ident = ident.to_string().to_case(Case::Pascal);
        let ident = Ident::new(ident.as_str(), span);
        let sum_of_field_offsets = field_names.iter().take(i).map(|field| {
            let ty = &field.ty;
            quote! {
                <#ty as #crate_name::Encoder<#ty>>::HEADER_SIZE
            }
        });
        let ty = &field.ty;
        quote! {
            type #ident = #crate_name::FieldEncoder<#ty, { 0 #( +#sum_of_field_offsets )* }>;
        }
    });
    let struct_name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let i_struct_name = format_ident!("I{}", ast.ident);
    let output = quote! {
        impl #impl_generics #crate_name::Encoder<#struct_name #type_generics> for #struct_name #type_generics #where_clause {
            const HEADER_SIZE: usize = 0 #( + #header_sizes)*;
            fn encode<W: #crate_name::WritableBuffer>(&self, encoder: &mut W, mut field_offset: usize) {
                #( #encode_types; )*
            }
            fn decode_header(decoder: &mut #crate_name::BufferDecoder, mut field_offset: usize, result: &mut #struct_name #type_generics) -> (usize, usize) {
                #( #decode_types; )*
                (0, 0)
            }
        }
        pub trait #i_struct_name {
            #( #impl_types )*
        }
        impl #impl_generics #i_struct_name for #struct_name #type_generics {
            #( #impl_defs )*
        }
    };
    TokenStream::from(output)
}

#[proc_macro_derive(Codec)]
pub fn codec_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_derive_codec(&ast)
}

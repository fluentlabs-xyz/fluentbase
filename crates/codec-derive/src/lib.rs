use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, parse_quote, Data, DeriveInput, Fields, Ident};

struct FieldInfo {
    ident: Ident,
    ty: syn::Type,
}

struct CodecStruct {
    struct_name: Ident,
    generics: syn::Generics,
    fields: Vec<FieldInfo>,
}

impl CodecStruct {
    fn parse(ast: &DeriveInput) -> Self {
        let data_struct = match &ast.data {
            Data::Struct(s) => s,
            _ => panic!("`Codec` can only be derived for structs"),
        };

        let named_fields = match &data_struct.fields {
            Fields::Named(named_fields) => named_fields,
            _ => panic!("`Codec` can only be derived for structs with named fields"),
        };

        let fields = named_fields
            .named
            .iter()
            .map(|field| {
                let ident = field.ident.as_ref().unwrap().clone();
                let ty = field.ty.clone();
                FieldInfo { ident, ty }
            })
            .collect();

        CodecStruct {
            struct_name: ast.ident.clone(),
            generics: ast.generics.clone(),
            fields,
        }
    }

    fn generate_impl(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let struct_name = &self.struct_name;
        let mut generics = self.generics.clone();

        let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();
        let crate_name = if crate_name == "fluentbase-codec" {
            quote! { crate }
        } else if crate_name == "fluentbase-sdk"
            || crate_name == "fluentbase-types"
            || crate_name == "fluentbase-runtime"
        {
            quote! { ::fluentbase_codec }
        } else {
            quote! { ::fluentbase_sdk::codec }
        };

        let has_custom_generics = !generics.params.is_empty();

        let needs_b = !generics
            .params
            .iter()
            .any(|p| matches!(p, syn::GenericParam::Type(t) if t.ident == "B"));
        let needs_align = !generics
            .params
            .iter()
            .any(|p| matches!(p, syn::GenericParam::Const(c) if c.ident == "ALIGN"));

        if needs_b {
            generics
                .params
                .push(parse_quote!(B: #crate_name::byteorder::ByteOrder));
        }
        if needs_align {
            generics.params.push(parse_quote!(const ALIGN: usize));
        }

        // Add bounds for each field type requiring Encoder implementation
        let encoder_bounds: Vec<syn::WherePredicate> = self
            .fields
            .iter()
            .map(|field| {
                let ty = &field.ty;
                parse_quote!(#ty: #crate_name::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>)
            })
            .collect();

        let where_clause = if let Some(mut where_clause) = generics.where_clause.clone() {
            where_clause.predicates.extend(encoder_bounds);
            where_clause
        } else {
            parse_quote!(where #(#encoder_bounds),*)
        };

        let (impl_generics, ty_generics, _) = generics.split_for_impl();

        let struct_name_with_ty = if has_custom_generics {
            quote! { #struct_name #ty_generics }
        } else {
            quote! { #struct_name }
        };

        let header_sizes = self.fields.iter().map(|field| {
            let ty = &field.ty;
            if sol_mode {
                quote! {
                    <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE
                }
            } else {
                quote! {
                    #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE)
                }
            }
        });

        let is_dynamic_expr = self.fields.iter().map(|field| {
            let ty = &field.ty;
            quote! {
                <#ty as #crate_name::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>>::IS_DYNAMIC
            }
        });

        let is_dynamic = quote! {
            false #( || #is_dynamic_expr)*
        };

        let encode_fields = self.fields.iter().map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;

            if sol_mode {
                quote! {
                    if <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                        <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::encode(&self.#ident, &mut tmp, current_offset)?;
                        current_offset += #crate_name::align_up::<ALIGN>(4);
                    } else {
                        <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::encode(&self.#ident, &mut tmp, current_offset)?;
                        current_offset += #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE);
                    }
                }
            } else {
                quote! {
                    <#ty as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::encode(&self.#ident, buf, current_offset)?;
                    current_offset += #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE);
                }
            }
        });

        let decode_fields = self.fields.iter().map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;

            if sol_mode {
                quote! {
                    let #ident = <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::decode(&mut tmp, current_offset)?;
                    current_offset += if <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                        32
                    } else {
                        #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE)
                    };
                }
            } else {
                quote! {
                    let #ident = <#ty as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::decode(buf, current_offset)?;
                    current_offset += #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE);
                }
            }
        });

        let aligned_header_size = if sol_mode {
            let sizes = self.fields.iter().map(|field| {
                let ty = &field.ty;
                let ts = quote! {
                    <#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>
                };
                quote! {
                    if #ts ::IS_DYNAMIC {
                        32
                    } else {
                        #crate_name::align_up::<ALIGN>(<#ty as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE)
                    }
                }
            });
            quote! { 0 #( + #sizes)* }
        } else {
            quote! { <Self as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE }
        };

        let struct_initialization = self.fields.iter().map(|field| {
            let ident = &field.ident;
            quote! { #ident }
        });

        let encode_impl = if sol_mode {
            quote! {
                let aligned_offset = #crate_name::align_up::<ALIGN>(offset);
                let is_dynamic = <Self as #crate_name::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC;
                let aligned_header_size = #aligned_header_size;

                if is_dynamic {
                    let buf_len = buf.len();
                    let offset = if buf_len == 0 { 32 } else { buf_len };
                    #crate_name::write_u32_aligned::<B, ALIGN>(buf, aligned_offset, offset as u32);
                }

                let mut tmp = #crate_name::bytes::BytesMut::zeroed(aligned_header_size);
                let mut current_offset = 0;

                #( #encode_fields )*

                buf.extend_from_slice(&tmp);
            }
        } else {
            quote! {
                let mut current_offset = #crate_name::align_up::<ALIGN>(offset);
                let header_size = <Self as #crate_name::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE;

                if buf.len() < current_offset + header_size {
                    buf.resize(current_offset + header_size, 0);
                }

                #( #encode_fields )*
            }
        };

        let decode_impl = if sol_mode {
            quote! {
                let mut aligned_offset = #crate_name::align_up::<ALIGN>(offset);

                let mut tmp = if #is_dynamic {
                    let offset = #crate_name::read_u32_aligned::<B, ALIGN>(&buf.chunk(), aligned_offset)? as usize;
                    &buf.chunk()[offset..]
                } else {
                    &buf.chunk()[aligned_offset..]
                };

                let mut current_offset = 0;

                #( #decode_fields )*
            }
        } else {
            quote! {
                let mut current_offset = #crate_name::align_up::<ALIGN>(offset);
                #( #decode_fields )*
            }
        };

        quote! {
            impl #impl_generics #crate_name::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>
                for #struct_name_with_ty
                #where_clause
            {
                const HEADER_SIZE: usize = 0 #( + #header_sizes)*;
                const IS_DYNAMIC: bool = #is_dynamic;

                fn encode(&self, buf: &mut #crate_name::bytes::BytesMut, offset: usize) -> Result<(), #crate_name::CodecError> {
                    #encode_impl
                    Ok(())
                }

                fn decode(buf: &impl #crate_name::bytes::Buf, offset: usize) -> Result<Self, #crate_name::CodecError> {
                    #decode_impl

                    Ok(#struct_name {
                        #( #struct_initialization ),*
                    })
                }

                fn partial_decode(buffer: &impl #crate_name::bytes::Buf, offset: usize) -> Result<(usize, usize), #crate_name::CodecError> {
                    Ok((0,0))
                }
            }
        }
    }
}

impl ToTokens for CodecStruct {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        // Generate implementations for both true and false IS_STATIC
        let sol_impl_static = self.generate_impl(true, true);
        let sol_impl_dynamic = self.generate_impl(true, false);
        let wasm_impl_static = self.generate_impl(false, true);
        let wasm_impl_dynamic = self.generate_impl(false, false);

        tokens.extend(quote! {
            #sol_impl_static
            #sol_impl_dynamic
            #wasm_impl_static
            #wasm_impl_dynamic
        });
    }
}

#[proc_macro_derive(Codec)]
pub fn codec_macro_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let codec_struct = CodecStruct::parse(&ast);
    quote! {
        #codec_struct
    }
    .into()
}

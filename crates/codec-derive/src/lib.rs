//! Procedural macros for deriving `fluentbase_codec::Codec` on Rust structs.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{
    parse_macro_input,
    parse_quote,
    Data,
    DeriveInput,
    Fields,
    GenericParam,
    Ident,
    Type,
    WhereClause,
    WherePredicate,
};

/// Holds information about a struct field
struct FieldInfo {
    ident: Ident,
    ty: Type,
}

/// Represents a struct for which we are deriving Codec
struct CodecStruct {
    struct_name: Ident,
    generics: syn::Generics,
    fields: Vec<FieldInfo>,
}

impl CodecStruct {
    /// Parse the DeriveInput to extract struct information
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

    /// Detect the crate path for the codec library
    fn get_crate_path() -> TokenStream2 {
        let crate_name = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
        if crate_name == "fluentbase-codec"
            || crate_name == "fluentbase-sdk"
            || crate_name == "fluentbase-types"
            || crate_name == "fluentbase-runtime"
        {
            quote! { ::fluentbase_codec }
        } else {
            quote! { ::fluentbase_sdk::codec }
        }
    }

    /// Prepare generics by adding necessary type and const parameters
    fn prepare_generics(&self, original_generics: &syn::Generics) -> syn::Generics {
        let mut generics = original_generics.clone();
        let crate_path = Self::get_crate_path();

        // Check if B and ALIGN parameters already exist
        let needs_b = !generics
            .params
            .iter()
            .any(|p| matches!(p, GenericParam::Type(t) if t.ident == "B"));
        let needs_align = !generics
            .params
            .iter()
            .any(|p| matches!(p, GenericParam::Const(c) if c.ident == "ALIGN"));

        // Add them if needed
        if needs_b {
            generics
                .params
                .push(parse_quote!(B: #crate_path::byteorder::ByteOrder));
        }
        if needs_align {
            generics.params.push(parse_quote!(const ALIGN: usize));
        }

        generics
    }

    /// Add where clause predicates for the Encoder trait bound on each field
    fn add_encoder_bounds(
        &self,
        generics: &syn::Generics,
        sol_mode: bool,
        is_static: bool,
    ) -> WhereClause {
        let crate_path = Self::get_crate_path();

        // Create bounds for each field requiring Encoder implementation
        let encoder_bounds: Vec<WherePredicate> = self
            .fields
            .iter()
            .map(|field| {
                let ty = &field.ty;
                parse_quote!(#ty: #crate_path::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>)
            })
            .collect();

        // Add them to existing where clause or create a new one
        if let Some(mut where_clause) = generics.where_clause.clone() {
            where_clause.predicates.extend(encoder_bounds);
            where_clause
        } else {
            parse_quote!(where #(#encoder_bounds),*)
        }
    }

    /// Generate expression for checking if a field type is dynamic
    fn generate_is_dynamic_expr(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        let is_dynamic_expr = self.fields.iter().map(|field| {
            let ty = &field.ty;
            quote! {
                <#ty as #crate_path::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>>::IS_DYNAMIC
            }
        });

        quote! {
            false #( || #is_dynamic_expr)*
        }
    }

    /// Generate expression for calculating header size
    fn generate_header_size_expr(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        let header_sizes = self.fields.iter().map(|field| {
            let ty = &field.ty;
            if sol_mode {
                quote! {
                    <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE
                }
            } else {
                quote! {
                    #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE)
                }
            }
        });

        quote! {
            0 #( + #header_sizes)*
        }
    }

    /// Generate code for the aligned_header_size expression
    fn generate_aligned_header_size(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        if sol_mode {
            let sizes = self.fields.iter().map(|field| {
                let ty = &field.ty;
                let ts = quote! {
                    <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>
                };
                quote! {
                    if #ts ::IS_DYNAMIC {
                        32
                    } else {
                        #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE)
                    }
                }
            });
            quote! { 0 #( + #sizes)* }
        } else {
            quote! { <Self as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE }
        }
    }

    /// Generate encode implementation for fields
    fn generate_encode_fields(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        let encode_fields = self.fields.iter().map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;

            if sol_mode {
                quote! {
                    if <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                        <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::encode(&self.#ident, &mut tail, tail_offset)?;
                        tail_offset += #crate_path::align_up::<ALIGN>(4);
                    } else {
                        <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::encode(&self.#ident, &mut tail, tail_offset)?;
                        tail_offset += #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE);
                    }
                }
            } else {
                quote! {
                    <#ty as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::encode(&self.#ident, buf, current_offset)?;
                    current_offset += #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE);
                }
            }
        });

        quote! { #(#encode_fields)* }
    }

    /// Generate decode implementation for fields
    fn generate_decode_fields(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        let decode_fields = self.fields.iter().map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;

            if sol_mode {
                quote! {
                    let #ident = <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::decode(&mut tmp, current_offset)?;
                    current_offset += if <#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                        32
                    } else {
                        #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE)
                    };
                }
            } else {
                quote! {
                    let #ident = <#ty as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::decode(buf, current_offset)?;
                    current_offset += #crate_path::align_up::<ALIGN>(<#ty as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE);
                }
            }
        });

        quote! { #(#decode_fields)* }
    }

    /// Generate encode method implementation
    fn generate_encode_impl(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();
        let encode_fields = self.generate_encode_fields(sol_mode, is_static);
        let aligned_header_size = self.generate_aligned_header_size(sol_mode, is_static);

        if sol_mode {
            quote! {
                let aligned_offset = #crate_path::align_up::<ALIGN>(offset);
                let is_dynamic = <Self as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC;
                let aligned_header_size = #aligned_header_size;

                let mut tail = if is_dynamic {
                    let buf_len = buf.len();
                    let offset = if buf_len != 0 { buf_len } else { 32 };
                    #crate_path::write_u32_aligned::<B, ALIGN>(buf, aligned_offset, offset as u32);
                    if buf.len() < aligned_header_size + offset {
                        buf.resize(aligned_header_size + offset, 0);
                    }
                    buf.split_off(offset)
                } else {
                    if buf.len() < aligned_offset + aligned_header_size {
                        buf.resize(aligned_offset + aligned_header_size, 0);
                    }
                    buf.split_off(aligned_offset)
                };
                let mut tail_offset = 0;

                #encode_fields

                buf.unsplit(tail);
                Ok(())
            }
        } else {
            quote! {
                let mut current_offset = #crate_path::align_up::<ALIGN>(offset);
                let header_size = <Self as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE;

                if buf.len() < current_offset + header_size {
                    buf.resize(current_offset + header_size, 0);
                }

                #encode_fields
                Ok(())
            }
        }
    }

    /// Generate decode method implementation
    fn generate_decode_impl(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();
        let decode_fields = self.generate_decode_fields(sol_mode, is_static);
        let struct_name = &self.struct_name;

        // Get field identifiers for struct initialization
        let struct_initialization = self.fields.iter().map(|field| {
            let ident = &field.ident;
            quote! { #ident }
        });

        let decode_body = if sol_mode {
            quote! {
                let mut aligned_offset = #crate_path::align_up::<ALIGN>(offset);

                let mut tmp = if <Self as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                    let offset = #crate_path::read_u32_aligned::<B, ALIGN>(&buf.chunk(), aligned_offset)? as usize;
                    &buf.chunk()[offset..]
                } else {
                    &buf.chunk()[aligned_offset..]
                };

                let mut current_offset = 0;

                #decode_fields
            }
        } else {
            quote! {
                let mut current_offset = #crate_path::align_up::<ALIGN>(offset);
                #decode_fields
            }
        };

        quote! {
            #decode_body

            Ok(#struct_name {
                #( #struct_initialization ),*
            })
        }
    }

    /// Generate partial_decode method implementation for struct data
    fn generate_partial_decode_impl(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let crate_path = Self::get_crate_path();

        if sol_mode {
            quote! {
                // For Solidity ABI encoding
                let aligned_offset = #crate_path::align_up::<ALIGN>(offset);

                if <Self as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::IS_DYNAMIC {
                    // For dynamic structs, read the offset pointer
                    let data_offset = #crate_path::read_u32_aligned::<B, ALIGN>(&buffer.chunk(), aligned_offset)? as usize;
                    // Return the actual data location and the header size
                    Ok((data_offset, <Self as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE))
                } else {
                    // For static structs, return current offset and header size
                    Ok((aligned_offset, <Self as #crate_path::Encoder<B, ALIGN, {true}, {#is_static}>>::HEADER_SIZE))
                }
            }
        } else {
            quote! {
                // For Compact ABI encoding
                let aligned_offset = #crate_path::align_up::<ALIGN>(offset);
                // Return the current offset and the struct's header size
                Ok((aligned_offset, <Self as #crate_path::Encoder<B, ALIGN, {false}, {#is_static}>>::HEADER_SIZE))
            }
        }
    }

    /// Generate the complete trait implementation for a specific mode and static/dynamic setting
    fn generate_impl(&self, sol_mode: bool, is_static: bool) -> TokenStream2 {
        let struct_name = &self.struct_name;
        let crate_path = Self::get_crate_path();

        let generics = self.prepare_generics(&self.generics);
        let where_clause = self.add_encoder_bounds(&generics, sol_mode, is_static);
        let (impl_generics, ty_generics, _) = generics.split_for_impl();

        let has_custom_generics = !self.generics.params.is_empty();
        let struct_name_with_ty = if has_custom_generics {
            quote! { #struct_name #ty_generics }
        } else {
            quote! { #struct_name }
        };

        let header_size = self.generate_header_size_expr(sol_mode, is_static);
        let is_dynamic = self.generate_is_dynamic_expr(sol_mode, is_static);

        let encode_impl = self.generate_encode_impl(sol_mode, is_static);
        let decode_impl = self.generate_decode_impl(sol_mode, is_static);
        let partial_decode_impl = self.generate_partial_decode_impl(sol_mode, is_static);

        quote! {
            impl #impl_generics #crate_path::Encoder<B, ALIGN, {#sol_mode}, {#is_static}>
                for #struct_name_with_ty
                #where_clause
            {
                const HEADER_SIZE: usize = #header_size;
                const IS_DYNAMIC: bool = #is_dynamic;

                fn encode(&self, buf: &mut #crate_path::bytes::BytesMut, offset: usize) -> Result<(), #crate_path::CodecError> {
                    #encode_impl
                }

                fn decode(buf: &impl #crate_path::bytes::Buf, offset: usize) -> Result<Self, #crate_path::CodecError> {
                    #decode_impl
                }

                fn partial_decode(buffer: &impl #crate_path::bytes::Buf, offset: usize) -> Result<(usize, usize), #crate_path::CodecError> {
                    #partial_decode_impl
                }
            }
        }
    }
}

impl ToTokens for CodecStruct {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
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

/// Derive macro for implementing Codec trait for structs
#[proc_macro_derive(Codec, attributes(codec))]
pub fn codec_macro_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let codec_struct = CodecStruct::parse(&ast);
    quote! {
        #codec_struct
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use proc_macro2::TokenStream;
    use syn::parse_quote;

    fn get_generated_code(input: TokenStream) -> String {
        let ast = syn::parse2::<DeriveInput>(input).unwrap();
        let codec_struct = CodecStruct::parse(&ast);
        let tokens = quote! { #codec_struct };
        prettyplease::unparse(&syn::parse2::<syn::File>(tokens).unwrap())
    }

    #[test]
    fn test_simple_struct() {
        let input = parse_quote! {
            #[derive(Codec, Default, Debug, PartialEq)]
            struct TestStruct {
                bool_val: bool,
                bytes_val: Bytes,
                vec_val: Vec<u32>,
            }
        };

        assert_snapshot!("simple_struct", get_generated_code(input));
    }

    #[test]
    fn test_generic_struct() {
        let input = parse_quote! {
            #[derive(Codec, Default, Debug, PartialEq)]
            struct GenericStruct<T>
            where
                T: Clone + Default,
            {
                field1: T,
                field2: Vec<T>,
            }
        };

        assert_snapshot!("generic_struct", get_generated_code(input));
    }

    #[test]
    fn test_single_field_struct() {
        let input = parse_quote! {
            #[derive(Codec, Default, Debug, PartialEq)]
            struct SingleFieldStruct {
                value: u64,
            }
        };

        assert_snapshot!("single_field_struct", get_generated_code(input));
    }

    #[test]
    fn test_empty_struct() {
        let input = parse_quote! {
            #[derive(Codec, Default, Debug, PartialEq)]
            struct EmptyStruct {}
        };

        assert_snapshot!("empty_struct", get_generated_code(input));
    }
}

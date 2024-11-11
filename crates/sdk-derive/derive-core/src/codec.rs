use crate::mode::RouterMode;
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::Index;

/// Generates codec implementation for route methods.
pub struct CodecGenerator<'a> {
    /// Method name
    fn_name: &'a str,
    /// Function selector
    function_id: &'a [u8; 4],
    /// Function signature
    signature: &'a str,
    /// Input parameter types
    input_types: Vec<&'a syn::Type>,
    /// Return types
    return_types: Vec<&'a syn::Type>,
    /// Router mode
    mode: &'a RouterMode,
}

impl<'a> CodecGenerator<'a> {
    /// Creates a new CodecGenerator instance.
    pub fn new(
        fn_name: &'a str,
        function_id: &'a [u8; 4],
        signature: &'a str,
        input_types: Vec<&'a syn::Type>,
        return_types: Vec<&'a syn::Type>,
        mode: &'a RouterMode,
    ) -> Self {
        Self {
            fn_name,
            function_id,
            signature,
            input_types,
            return_types,
            mode,
        }
    }

    /// Determines crate names based on the current package
    fn determine_crate_name(&self) -> TokenStream2 {
        let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();
        let crate_name = if crate_name == "fluentbase-codec" {
            quote! { crate }
        } else if crate_name == "fluentbase-sdk"
            || crate_name == "fluentbase-types"
            || crate_name == "fluentbase-runtime"
        {
            quote! { fluentbase_codec }
        } else {
            quote! { fluentbase_sdk::codec }
        };

        crate_name
    }

    /// Determines the codec type based on mode.
    fn get_codec_type(&self) -> TokenStream2 {
        let crate_name = self.determine_crate_name();
        match self.mode {
            RouterMode::Solidity => quote! { #crate_name::encoder::SolidityABI },
            RouterMode::Fluent => quote! { #crate_name::encoder::FluentABI },
        }
    }

    /// Generates the complete codec implementation.
    pub fn generate(&self) -> TokenStream2 {
        let struct_names = self.generate_struct_names();
        let codec_type = self.get_codec_type();
        let args_types = self.generate_args_types(&struct_names);
        let (call_offset_handling, return_offset_handling) = self.generate_offset_handling(
            &codec_type,
            &struct_names.call_args,
            &struct_names.return_args,
        );
        let field_encoders = self.generate_field_encoders();

        self.generate_implementation(
            struct_names,
            codec_type,
            args_types,
            (call_offset_handling, return_offset_handling),
            field_encoders,
        )
    }

    /// Generates struct and type names.
    fn generate_struct_names(&self) -> StructNames {
        let pascal_name = self.fn_name.to_case(Case::Pascal);
        StructNames {
            call: format_ident!("{}Call", pascal_name),
            call_args: format_ident!("{}CallArgs", pascal_name),
            call_target: format_ident!("{}CallTarget", pascal_name),
            return_: format_ident!("{}Return", pascal_name),
            return_args: format_ident!("{}ReturnArgs", pascal_name),
            return_target: format_ident!("{}ReturnTarget", pascal_name),
        }
    }

    /// Generates argument types for input and output.
    fn generate_args_types(&self, _names: &StructNames) -> ArgsTypes {
        let input_types = self.input_types.clone();
        let return_types = self.return_types.clone();
        let input = if self.input_types.is_empty() {
            quote! { () }
        } else {
            quote! { (#(#input_types,)*) }
        };

        let output = if self.return_types.is_empty() {
            quote! { () }
        } else {
            quote! { (#(#return_types,)*) }
        };

        ArgsTypes { input, output }
    }

    /// Generates field encoders for input and output fields.
    fn generate_field_encoders(&self) -> FieldEncoders {
        let encode_input_fields = (0..self.input_types.len())
            .map(|i| {
                let index = Index::from(i);
                quote! { self.0.clone().#index }
            })
            .collect();

        let encode_output_fields = (0..self.return_types.len())
            .map(|i| {
                let index = Index::from(i);
                quote! { self.0.clone().#index }
            })
            .collect();

        FieldEncoders {
            encode_input_fields,
            encode_output_fields,
        }
    }

    /// Generates dynamic offset handling code.
    fn generate_offset_handling(
        &self,
        codec_type: &TokenStream2,
        call_args: &syn::Ident,
        return_args: &syn::Ident,
    ) -> (OffsetHandling, OffsetHandling) {
        let (call_encode_offset, call_decode_offset) = match self.mode {
            RouterMode::Solidity => (
                quote! {
                    if #codec_type::<#call_args>::is_dynamic() {
                        encoded_args[32..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#call_args>::is_dynamic() {
                        ::fluentbase_sdk::U256::from(32).to_be_bytes::<32>().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
            RouterMode::Fluent => (
                quote! {
                    if #codec_type::<#call_args>::is_dynamic() {
                        encoded_args[4..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#call_args>::is_dynamic() {
                        (4_u32).to_le_bytes().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
        };

        let (return_encode_offset, return_decode_offset) = match self.mode {
            RouterMode::Solidity => (
                quote! {
                    if #codec_type::<#return_args>::is_dynamic() {
                        encoded_args[32..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#return_args>::is_dynamic() {
                        ::fluentbase_sdk::U256::from(32).to_be_bytes::<32>().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
            RouterMode::Fluent => (
                quote! {
                    if #codec_type::<#return_args>::is_dynamic() {
                        encoded_args[4..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#return_args>::is_dynamic() {
                        (4_u32).to_le_bytes().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
        };

        (
            OffsetHandling {
                encode_offset: call_encode_offset,
                decode_offset: call_decode_offset,
            },
            OffsetHandling {
                encode_offset: return_encode_offset,
                decode_offset: return_decode_offset,
            },
        )
    }

    /// Generates the final implementation code.
    fn generate_implementation(
        &self,
        names: StructNames,
        codec_type: TokenStream2,
        args_types: ArgsTypes,
        offset: (OffsetHandling, OffsetHandling),
        encoders: FieldEncoders,
    ) -> TokenStream2 {
        let crate_name = self.determine_crate_name();
        let function_id = self.function_id;
        let signature = self.signature;

        let StructNames {
            call,
            call_args,
            call_target,
            return_,
            return_args,
            return_target,
        } = names;

        let ArgsTypes { input, output } = args_types;
        let (
            OffsetHandling {
                encode_offset: call_encode_offset,
                decode_offset: call_decode_offset,
            },
            OffsetHandling {
                encode_offset: return_encode_offset,
                decode_offset: return_decode_offset,
            },
        ) = offset;
        let FieldEncoders {
            encode_input_fields,
            encode_output_fields,
        } = encoders;

        quote! {
            pub type #call_args = #input;
            pub struct #call(#call_args);

            impl #call {
                pub const SELECTOR: [u8; 4] = [#(#function_id,)*];
                pub const SIGNATURE: &'static str = #signature;

                pub fn new(args: #call_args) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> #crate_name::bytes::Bytes {
                    let mut buf = #crate_name::bytes::BytesMut::new();
                    #codec_type::encode(&(#(#encode_input_fields,)*), &mut buf, 0).unwrap();
                    let encoded_args = buf.freeze();
                    let clean_args = #call_encode_offset;

                    Self::SELECTOR.iter().copied().chain(clean_args).collect()
                }

                pub fn decode(buf: &impl #crate_name::bytes::Buf) -> ::core::result::Result<Self,  #crate_name::CodecError> {
                    use #crate_name::bytes::BufMut;
                    let dynamic_offset = #call_decode_offset;
                    let mut combined_buf = #crate_name::bytes::BytesMut::new();
                    combined_buf.put_slice(&dynamic_offset);
                    combined_buf.put_slice(buf.chunk());

                    let args = #codec_type::<#call_args>::decode(&combined_buf.freeze(), 0)?;
                    Ok(Self(args))
                }
            }

            impl ::core::ops::Deref for #call {
                type Target = #call_args;
                fn deref(&self) -> &Self::Target { &self.0 }
            }

            pub type #call_target = <#call as ::core::ops::Deref>::Target;

            pub type #return_args = #output;

            #[derive(Debug)]
            pub struct #return_(#return_args);

            impl #return_ {
                pub fn new(args: #return_args) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> #crate_name::bytes::Bytes {
                    let mut buf = #crate_name::bytes::BytesMut::new();
                    #codec_type::encode(&(#(#encode_output_fields,)*), &mut buf, 0).unwrap();
                    let encoded_args = buf.freeze();
                    let clean_args = #return_encode_offset;

                    clean_args.into()
                }

                pub fn decode(buf: &impl #crate_name::bytes::Buf) -> ::core::result::Result<Self,  #crate_name::CodecError> {
                    use #crate_name::bytes::BufMut;
                    let dynamic_offset = #return_decode_offset;
                    let mut combined_buf = #crate_name::bytes::BytesMut::new();
                    combined_buf.put_slice(&dynamic_offset);
                    combined_buf.put_slice(buf.chunk());

                    let args = #codec_type::<#return_args>::decode(&combined_buf.freeze(), 0)?;
                    Ok(Self(args))
                }
            }

            impl ::core::ops::Deref for #return_ {
                type Target = #return_args;
                fn deref(&self) -> &Self::Target { &self.0 }
            }

            pub type #return_target = <#return_ as ::core::ops::Deref>::Target;
        }
    }
}

/// Helper struct for storing generated struct names
#[derive(Clone)]
struct StructNames {
    call: syn::Ident,
    call_args: syn::Ident,
    call_target: syn::Ident,
    return_: syn::Ident,
    return_args: syn::Ident,
    return_target: syn::Ident,
}

/// Helper struct for storing generated argument types
struct ArgsTypes {
    input: TokenStream2,
    output: TokenStream2,
}

/// Helper struct for storing offset handling code
struct OffsetHandling {
    encode_offset: TokenStream2,
    decode_offset: TokenStream2,
}

/// Helper struct for storing field encoders
struct FieldEncoders {
    encode_input_fields: Vec<TokenStream2>,
    encode_output_fields: Vec<TokenStream2>,
}

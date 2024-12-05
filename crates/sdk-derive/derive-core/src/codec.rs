use crate::{abi::FunctionABI, mode::Mode};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::Result;

/// Generator for encoding/decoding ABI parameters for function calls and returns
#[derive(Debug)]
pub struct CodecGenerator<'a> {
    /// Core ABI representation
    abi: &'a FunctionABI,
    /// Function selector - 4 bytes
    function_id: &'a [u8; 4],
    /// Router mode configuration
    mode: &'a Mode,
}

impl<'a> CodecGenerator<'a> {
    /// Creates a new CodecGenerator instance
    pub fn new(abi: &'a FunctionABI, function_id: &'a [u8; 4], mode: &'a Mode) -> Self {
        Self {
            abi,
            function_id,
            mode,
        }
    }

    /// Determines the appropriate crate path for codec implementations
    fn get_crate_path(&self) -> TokenStream2 {
        match std::env::var("CARGO_PKG_NAME").as_deref() {
            Ok("fluentbase-codec") => quote! { crate },
            Ok("fluentbase-sdk" | "fluentbase-types" | "fluentbase-runtime") => {
                quote! { fluentbase_codec }
            }
            _ => quote! { fluentbase_sdk::codec },
        }
    }

    /// Gets the appropriate codec type based on router mode
    fn get_codec_type(&self) -> TokenStream2 {
        let crate_name = self.get_crate_path();
        match self.mode {
            Mode::Solidity => quote! { #crate_name::encoder::SolidityABI },
            Mode::Fluent => quote! { #crate_name::encoder::FluentABI },
        }
    }

    /// Generates the complete codec implementation including both call and return types
    pub fn generate(&self) -> Result<TokenStream2> {
        let call_codec = self.generate_call_codec()?;
        let return_codec = self.generate_return_codec()?;

        Ok(quote! {
            #call_codec
            #return_codec
        })
    }

    /// Generates offset handling code based on mode and type.
    ///
    /// Returns (encode_offset, decode_offset) token streams.
    /// Different offsets are used for Solidity (32 bytes) and Fluent (4 bytes) modes.
    fn generate_offset_handling(
        &self,
        codec_type: &TokenStream2,
        type_args: &syn::Ident,
    ) -> (TokenStream2, TokenStream2) {
        match self.mode {
            Mode::Solidity => (
                quote! {
                    if #codec_type::<#type_args>::is_dynamic() {
                        encoded_args[32..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#type_args>::is_dynamic() {
                        ::fluentbase_sdk::U256::from(32).to_be_bytes::<32>().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
            Mode::Fluent => (
                quote! {
                    if #codec_type::<#type_args>::is_dynamic() {
                        encoded_args[4..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    }
                },
                quote! {
                    if #codec_type::<#type_args>::is_dynamic() {
                        (4_u32).to_le_bytes().to_vec()
                    } else {
                        ::alloc::vec::Vec::new()
                    }
                },
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Generates common struct implementation for both call and return types.
    fn generate_struct_impl(
        &self,
        struct_name: &syn::Ident,
        args_type: &syn::Ident,
        field_indices: impl Iterator<Item = syn::Index> + Clone,
        crate_name: &TokenStream2,
        codec_type: &TokenStream2,
        encode_offset: &TokenStream2,
        decode_offset: &TokenStream2,
        is_call: bool,
    ) -> TokenStream2 {
        let function_id = self.function_id;
        let signature = self.abi.signature();

        // Generate additional implementation for call structs
        let struct_impl = if is_call {
            quote! {
                pub const SELECTOR: [u8; 4] = [#(#function_id,)*];
                pub const SIGNATURE: &'static str = #signature;
            }
        } else {
            quote! {}
        };

        // Generate appropriate encode result based on type
        let encode_result = if is_call {
            quote! { Self::SELECTOR.iter().copied().chain(clean_args).collect() }
        } else {
            quote! { clean_args.into() }
        };

        quote! {
            impl #struct_name {
                #struct_impl

                pub fn new(args: #args_type) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> #crate_name::bytes::Bytes {
                    let mut buf = #crate_name::bytes::BytesMut::new();
                    #codec_type::encode(&(#(self.0.#field_indices.clone(),)*), &mut buf, 0)
                        .expect("Failed to encode values");

                    let encoded_args = buf.freeze();
                    let clean_args = #encode_offset;

                    #encode_result
                }

                pub fn decode(buf: &impl #crate_name::bytes::Buf) -> Result<Self, #crate_name::CodecError> {
                    use #crate_name::bytes::BufMut;

                    let mut combined_buf = #crate_name::bytes::BytesMut::new();
                    combined_buf.put_slice(&#decode_offset);
                    combined_buf.put_slice(buf.chunk());

                    let args = #codec_type::<#args_type>::decode(&combined_buf.freeze(), 0)?;
                    Ok(Self(args))
                }
            }

            impl ::core::ops::Deref for #struct_name {
                type Target = #args_type;
                fn deref(&self) -> &Self::Target { &self.0 }
            }
        }
    }

    /// Generates codec implementation for function call parameters
    fn generate_call_codec(&self) -> Result<TokenStream2> {
        let crate_name = self.get_crate_path();
        let codec_type = self.get_codec_type();

        // Generate struct and type names
        let name = self.abi.name.to_case(Case::Pascal);
        let call_struct = format_ident!("{}Call", name);
        let call_args = format_ident!("{}CallArgs", name);

        // Extract input types from ABI
        let inputs = self
            .abi
            .inputs
            .iter()
            .map(|p| {
                p.get_rust_type().ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "Failed to get Rust type for parameter",
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let field_indices = (0..self.abi.inputs.len()).map(syn::Index::from);

        let (encode_offset, decode_offset) = self.generate_offset_handling(&codec_type, &call_args);

        let struct_impl = self.generate_struct_impl(
            &call_struct,
            &call_args,
            field_indices,
            &crate_name,
            &codec_type,
            &encode_offset,
            &decode_offset,
            true,
        );

        Ok(quote! {
            pub type #call_args = (#(#inputs,)*);

            #[derive(Debug)]
            pub struct #call_struct(#call_args);

            #struct_impl
        })
    }

    /// Generates codec implementation for function return values
    fn generate_return_codec(&self) -> Result<TokenStream2> {
        let crate_name = self.get_crate_path();
        let codec_type = self.get_codec_type();

        // Generate struct and type names
        let name = self.abi.name.to_case(Case::Pascal);
        let return_struct = format_ident!("{}Return", name);
        let return_args = format_ident!("{}ReturnArgs", name);

        // Extract output types from ABI
        let outputs = self
            .abi
            .outputs
            .iter()
            .map(|p| {
                p.get_rust_type().ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "Failed to get Rust type for return value",
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let field_indices = (0..self.abi.outputs.len()).map(syn::Index::from);

        let (encode_offset, decode_offset) =
            self.generate_offset_handling(&codec_type, &return_args);

        let struct_impl = self.generate_struct_impl(
            &return_struct,
            &return_args,
            field_indices,
            &crate_name,
            &codec_type,
            &encode_offset,
            &decode_offset,
            false,
        );

        Ok(quote! {
            pub type #return_args = (#(#outputs,)*);

            #[derive(Debug)]
            pub struct #return_struct(#return_args);

            #struct_impl
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_codec_generation() {
        // Create test trait function
        let test_fn: syn::TraitItemFn = parse_quote! {
            fn test_function(&self, value: u64) -> String;
        };

        // Create FunctionABI from trait function
        let abi = FunctionABI::from_trait_fn(&test_fn).expect("Failed to create ABI");

        let function_id = [0x12, 0x34, 0x56, 0x78];
        let mode = Mode::Solidity;

        let generator = CodecGenerator::new(&abi, &function_id, &mode);
        let result = generator.generate();

        assert!(result.is_ok());
    }

    #[test]
    fn test_different_modes() {
        // Create test trait function
        let test_fn: syn::TraitItemFn = parse_quote! {
            fn test_function(&self, value: u64) -> String;
        };

        // Create FunctionABI from trait function
        let abi = FunctionABI::from_trait_fn(&test_fn).expect("Failed to create ABI");
        let function_id = [0x12, 0x34, 0x56, 0x78];

        // Test Solidity mode
        let gen_sol = CodecGenerator::new(&abi, &function_id, &Mode::Solidity);
        assert!(gen_sol.generate().is_ok());

        // Test Fluent mode
        let gen_fluent = CodecGenerator::new(&abi, &function_id, &Mode::Fluent);
        assert!(gen_fluent.generate().is_ok());
    }

    #[test]
    fn test_codec_generation_complex_types() {
        // Test with more complex parameter types
        let test_fn: syn::TraitItemFn = parse_quote! {
            fn complex_function(
                &self,
                uint_val: u256,
                bytes_val: Vec<u8>,
                string_val: String,
            ) -> (u64, String);
        };

        let abi = FunctionABI::from_trait_fn(&test_fn).expect("Failed to create ABI");
        let function_id = [0x12, 0x34, 0x56, 0x78];
        let mode = Mode::Solidity;

        let generator = CodecGenerator::new(&abi, &function_id, &mode);
        let result = generator.generate();

        assert!(result.is_ok());
    }

    #[test]
    fn test_codec_generation_no_params() {
        // Test function with no parameters
        let test_fn: syn::TraitItemFn = parse_quote! {
            fn no_params(&self) -> ();
        };

        let abi = FunctionABI::from_trait_fn(&test_fn).expect("Failed to create ABI");
        let function_id = [0x12, 0x34, 0x56, 0x78];
        let mode = Mode::Solidity;

        let generator = CodecGenerator::new(&abi, &function_id, &mode);
        let result = generator.generate();

        assert!(result.is_ok());
    }
}

use crate::{
    attr::Mode,
    method::{MethodLike, ParsedMethod},
};
use convert_case::{Case, Casing};
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{FnArg, Result, ReturnType, Type};

/// Handles generation of encoding/decoding code for contract function parameters and return values
///
/// This generator creates type definitions and implementations for both function calls and returns,
/// allowing for seamless serialization and deserialization of contract interactions.
#[derive(Debug)]
pub struct CodecGenerator<'a, T: MethodLike> {
    /// Parsed method containing function signature and metadata
    route: &'a ParsedMethod<T>,
    /// Router mode configuration (Solidity or Fluent)
    mode: Mode,
}

impl<'a, T: MethodLike> CodecGenerator<'a, T> {
    /// Creates a new CodecGenerator instance
    pub fn new(route: &'a ParsedMethod<T>, mode: &Mode) -> Self {
        Self { route, mode: *mode }
    }

    /// Generates the complete codec implementation
    pub fn generate(&self) -> Result<TokenStream2> {
        let base_name = self
            .route
            .parsed_signature()
            .rust_name()
            .to_case(Case::Pascal);

        let call_struct = format_ident!("{}Call", base_name);
        let call_args = format_ident!("{}CallArgs", base_name);
        let return_struct = format_ident!("{}Return", base_name);
        let return_args = format_ident!("{}ReturnArgs", base_name);

        let input_types = self.extract_input_types();
        let output_types = self.extract_output_types();

        let codec_impl = self.generate_codec_impl(
            &call_struct,
            &call_args,
            &return_struct,
            &return_args,
            &output_types,
        )?;

        Ok(quote! {
            pub type #call_args = (#(#input_types,)*);
            #[derive(Debug, Clone, PartialEq)]
            pub struct #call_struct(pub #call_args);

            pub type #return_args = (#(#output_types,)*);
            #[derive(Debug, Clone, PartialEq)]
            pub struct #return_struct(pub #return_args);

            // codec impl in a private const block
            #codec_impl
        })
    }

    /// Generates the codec implementation inside a const block
    fn generate_codec_impl(
        &self,
        call_struct: &Ident,
        call_args: &Ident,
        return_struct: &Ident,
        return_args: &Ident,
        output_types: &[&Type],
    ) -> Result<TokenStream2> {
        let crate_path = self.get_crate_path();
        let codec_type = self.get_codec_type();
        let selector = self.route.function_id();
        let signature = self.route.parsed_signature().function_abi()?.signature()?;

        let field_indices = (0..self
            .route
            .parsed_signature()
            .inputs_without_receiver()
            .len())
            .map(syn::Index::from)
            .collect::<Vec<_>>();

        let call_encode_offset = self.create_encode_offset_expr(&codec_type, call_args);
        let call_decode_offset = self.create_decode_offset_expr(&codec_type, call_args);
        let return_encode_offset = self.create_encode_offset_expr(&codec_type, return_args);
        let return_decode_offset = self.create_decode_offset_expr(&codec_type, return_args);

        let encode_call_impl = if field_indices.is_empty() {
            quote! {
                #codec_type::encode(&(), &mut buf, 0)
                    .expect("Failed to encode values");
            }
        } else if field_indices.len() == 1 {
            let index = &field_indices[0];
            quote! {
                let args = self.0.clone();
                #codec_type::encode(&(args.#index,), &mut buf, 0)
                    .expect("Failed to encode values");
            }
        } else {
            quote! {
                let args = self.0.clone();
                #codec_type::encode(&(#(args.#field_indices),*), &mut buf, 0)
                    .expect("Failed to encode values");
            }
        };

        let is_unit_type = match &self.route.parsed_signature().output {
            ReturnType::Default => true,
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple.elems.is_empty(),
                _ => false,
            },
        };

        let encode_return_impl = if is_unit_type {
            quote! {
                #codec_type::encode(&(), &mut buf, 0)
                    .expect("Failed to encode values");
            }
        } else if output_types.len() == 1 {
            quote! {
                let args = self.0.clone();
                #codec_type::encode(&(args.0,), &mut buf, 0)
                    .expect("Failed to encode values");
            }
        } else {
            quote! {
                let args = self.0.clone();
                #codec_type::encode(&args, &mut buf, 0)
                    .expect("Failed to encode values");
            }
        };

        let encode_method = if self.route.is_constructor() {
            quote! {
                /// Encodes without selector (for constructor)
                pub fn encode(&self) -> #crate_path::bytes::Bytes {
                    let mut buf = #crate_path::bytes::BytesMut::new();
                    #encode_call_impl
                    let encoded_args = buf.freeze();
                    let clean_args = #call_encode_offset;
                    clean_args.into()
                }
            }
        } else {
            quote! {
                /// Encodes with selector
                pub fn encode(&self) -> #crate_path::bytes::Bytes {
                    let mut buf = #crate_path::bytes::BytesMut::new();
                    #encode_call_impl
                    let encoded_args = buf.freeze();
                    let clean_args = #call_encode_offset;
                    Self::SELECTOR.iter().copied().chain(clean_args).collect()
                }
            }
        };

        Ok(quote! {
            const _: () = {
                impl #call_struct {
                    pub const SELECTOR: [u8; 4] = [#(#selector,)*];
                    pub const SIGNATURE: &'static str = #signature;

                    /// Creates a new call instance from arguments
                    pub fn new(args: #call_args) -> Self {
                        Self(args)
                    }

                    #encode_method

                    /// Decodes call arguments from bytes
                    pub fn decode(buf: &impl #crate_path::bytes::Buf) -> Result<Self, #crate_path::CodecError> {
                        use #crate_path::bytes::BufMut;

                        let mut combined_buf = #crate_path::bytes::BytesMut::new();
                        combined_buf.put_slice(&#call_decode_offset);
                        combined_buf.put_slice(buf.chunk());

                        let args = #codec_type::<#call_args>::decode(&combined_buf.freeze(), 0)?;
                        Ok(Self(args))
                    }
                }

                impl ::core::ops::Deref for #call_struct {
                    type Target = #call_args;
                    fn deref(&self) -> &Self::Target {
                        &self.0
                    }
                }

                impl #return_struct {
                    /// Creates a new return instance from values
                    pub fn new(args: #return_args) -> Self {
                        Self(args)
                    }

                    /// Encodes the return values to bytes
                    pub fn encode(&self) -> #crate_path::bytes::Bytes {
                        let mut buf = #crate_path::bytes::BytesMut::new();
                        #encode_return_impl
                        let encoded_args = buf.freeze();
                        let clean_args = #return_encode_offset;
                        clean_args.into()
                    }

                    /// Decodes return values from bytes
                    pub fn decode(buf: &impl #crate_path::bytes::Buf) -> Result<Self, #crate_path::CodecError> {
                        use #crate_path::bytes::BufMut;

                        let mut combined_buf = #crate_path::bytes::BytesMut::new();
                        combined_buf.put_slice(&#return_decode_offset);
                        combined_buf.put_slice(buf.chunk());

                        let args = #codec_type::<#return_args>::decode(&combined_buf.freeze(), 0)?;
                        Ok(Self(args))
                    }
                }

                impl ::core::ops::Deref for #return_struct {
                    type Target = #return_args;
                    fn deref(&self) -> &Self::Target {
                        &self.0
                    }
                }
            };
        })
    }

    /// Extracts input parameter types from the function signature
    fn extract_input_types(&self) -> Vec<&Type> {
        self.route
            .parsed_signature()
            .inputs()
            .iter()
            .filter_map(|arg| {
                if let FnArg::Typed(pat_type) = arg {
                    Some(&*pat_type.ty)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Extracts output types from the function signature
    fn extract_output_types(&self) -> Vec<&Type> {
        match &self.route.parsed_signature().output {
            ReturnType::Default => Vec::new(),
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple.elems.iter().collect(),
                ty => vec![ty],
            },
        }
    }

    /// Creates the encode offset expression based on the mode
    /// Important: Dynamic types need the first 32/4 bytes removed
    fn create_encode_offset_expr(
        &self,
        codec_type: &TokenStream2,
        type_name: &Ident,
    ) -> TokenStream2 {
        match self.mode {
            Mode::Solidity => quote! {
                if #codec_type::<#type_name>::is_dynamic() {
                    encoded_args[32..].to_vec()
                } else {
                    encoded_args.to_vec()
                }
            },
            Mode::Fluent => quote! {
                if #codec_type::<#type_name>::is_dynamic() {
                    encoded_args[4..].to_vec()
                } else {
                    encoded_args.to_vec()
                }
            },
        }
    }

    /// Creates the decode offset expression based on the mode
    fn create_decode_offset_expr(
        &self,
        codec_type: &TokenStream2,
        type_name: &Ident,
    ) -> TokenStream2 {
        match self.mode {
            Mode::Solidity => quote! {
                if #codec_type::<#type_name>::is_dynamic() {
                    ::fluentbase_sdk::U256::from(32).to_be_bytes::<32>().to_vec()
                } else {
                    ::alloc::vec::Vec::new()
                }
            },
            Mode::Fluent => quote! {
                if #codec_type::<#type_name>::is_dynamic() {
                    (4_u32).to_le_bytes().to_vec()
                } else {
                    ::alloc::vec::Vec::new()
                }
            },
        }
    }

    /// Determines the appropriate crate path to use for codec implementations
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
        let crate_path = self.get_crate_path();
        match self.mode {
            Mode::Solidity => quote! { #crate_path::encoder::SolidityABI },
            Mode::Fluent => quote! { #crate_path::encoder::FluentABI },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use prettyplease;
    use proc_macro2::TokenStream as TokenStream2;
    use syn::{parse_file, parse_quote, ImplItemFn};

    fn create_route(item: ImplItemFn) -> ParsedMethod<ImplItemFn> {
        ParsedMethod::from_ref(&item).unwrap()
    }

    fn create_generator(
        route: &ParsedMethod<ImplItemFn>,
        mode: Mode,
    ) -> CodecGenerator<ImplItemFn> {
        CodecGenerator::new(route, &mode)
    }

    fn format_tokens(tokens: TokenStream2) -> String {
        let parsed = parse_file(&tokens.to_string()).unwrap();
        prettyplease::unparse(&parsed)
    }

    #[test]
    fn test_simple_transfer_solidity() {
        let func: ImplItemFn = parse_quote! {
            fn transfer(&mut self, amount: u64, recipient: String) -> String {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("simple_transfer_solidity", formatted);
    }

    #[test]
    fn test_simple_transfer_fluent() {
        let func: ImplItemFn = parse_quote! {
            fn transfer(&mut self, amount: u64, recipient: String) -> String {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Fluent);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("simple_transfer_fluent", formatted);
    }

    #[test]
    fn test_complex_function() {
        let func: ImplItemFn = parse_quote! {
            fn complex_op(&mut self, data: Vec<u8>, pairs: Vec<(u64, String)>) -> (u64, String, Vec<u8>) {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("complex_function", formatted);
    }

    #[test]
    fn test_no_params() {
        let func: ImplItemFn = parse_quote! {
            fn no_params(&mut self) {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("no_params", formatted);
    }

    #[test]
    fn test_multiple_returns() {
        let func: ImplItemFn = parse_quote! {
            fn multi_return(&mut self) -> (u64, String, bool) {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("multiple_returns", formatted);
    }

    #[test]
    fn test_empty_return() {
        let func: ImplItemFn = parse_quote! {
            fn empty_return(&mut self) -> () {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("empty_return", formatted);
    }

    #[test]
    fn test_single_dynamic_param() {
        let func: ImplItemFn = parse_quote! {
            fn custom_greeting(&mut self, message: String) -> String {}
        };
        let route = create_route(func);
        let generator = create_generator(&route, Mode::Solidity);
        let result = generator.generate().unwrap();

        let formatted = format_tokens(result);
        assert_snapshot!("single_dynamic_param", formatted);
    }
}

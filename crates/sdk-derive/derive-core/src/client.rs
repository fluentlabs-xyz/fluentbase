use crate::{
    abi::FunctionABI,
    codec::CodecGenerator,
    function_id::FunctionIDAttribute,
    mode::Mode,
};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Attribute,
    Error,
    Result,
    TraitItem,
    TraitItemFn,
};

/// Client-side method generator for Router API
///
/// Used to generate client code for calling Solidity contracts
#[derive(Debug)]
pub struct ClientMethod {
    /// Core ABI representation
    abi: FunctionABI,
    /// Router mode configuration
    mode: Mode,
    /// Original trait method
    method: TraitItemFn,
}

impl ClientMethod {
    /// Creates a new ClientMethod from a trait method
    pub fn new(method: &TraitItemFn, mode: Mode) -> Result<Self> {
        let abi = FunctionABI::from_trait_fn(method).map_err(|e| {
            Error::new(
                method.span(),
                format!("Failed to convert method to ABI: {}", e),
            )
        })?;

        if let Some(attr) = method
            .attrs
            .iter()
            .find(|a| a.path().is_ident("function_id"))
        {
            let attr_value = attr.parse_args::<FunctionIDAttribute>()?;
            let attr_id = attr_value.function_id_bytes()?;
            let calculated_id = abi.selector();

            if attr_value.validate.unwrap_or(true) && attr_id != calculated_id {
                return Err(Error::new(
                    attr.span(),
                    format!(
                        "Function ID mismatch: expected 0x{}, got 0x{}\nExpected Rust method signature: {}",
                        hex::encode(calculated_id),
                        hex::encode(attr_id),
                        abi.signature()
                    ),
                ));
            }
        }

        Ok(Self {
            abi,
            mode,
            method: method.clone(),
        })
    }

    /// Generates codec implementations for encoding/decoding parameters
    fn generate_codecs(&self) -> TokenStream2 {
        let function_id = self.abi.selector();
        let codec_generator = CodecGenerator::new(&self.abi, &function_id, &self.mode);
        codec_generator
            .generate()
            .expect("Failed to generate codec")
    }

    /// Generates helper methods for encoding parameters and decoding results
    /// ```
    fn generate_helpers(&self) -> TokenStream2 {
        let fn_name = &self.method.sig.ident;
        let encode_name = format_ident!("encode_{}", fn_name);
        let decode_name = format_ident!("decode_{}", fn_name);

        let pascal_name = self.abi.name.to_case(Case::Pascal);
        let call_struct = format_ident!("{}Call", pascal_name);
        let return_struct = format_ident!("{}Return", pascal_name);

        let encode_fn = {
            let params = self.abi.inputs.iter().map(|param| {
                let name = &param
                    .fn_arg
                    .as_ref()
                    .expect("Parameter must have function argument info")
                    .name;
                let name = format_ident!("{}", name);
                let ty = &param.fn_arg.as_ref().unwrap().ty;
                quote! { #name: #ty }
            });

            let param_names = self.abi.inputs.iter().map(|param| {
                let name = &param
                    .fn_arg
                    .as_ref()
                    .expect("Parameter must have function argument info")
                    .name;
                format_ident!("{}", name)
            });

            quote! {
                pub fn #encode_name(
                    &self,
                    #(#params,)*
                ) -> fluentbase_sdk::Bytes {
                    #call_struct::new((#(#param_names,)*)).encode().into()
                }
            }
        };

        let decode_fn = match self.abi.outputs.len() {
            0 => quote! {
                pub fn #decode_name(
                    &self,
                    _output: fluentbase_sdk::Bytes
                ) {
                    // No return value
                }
            },
            1 => {
                let return_ty = &self.abi.outputs[0].fn_arg.as_ref().unwrap().ty;
                quote! {
                    pub fn #decode_name(
                        &self,
                        output: fluentbase_sdk::Bytes
                    ) -> #return_ty {
                        #return_struct::decode(&output)
                            .expect("failed to decode result")
                            .0.0
                    }
                }
            }
            _ => {
                let return_types = self
                    .abi
                    .outputs
                    .iter()
                    .map(|param| &param.fn_arg.as_ref().unwrap().ty);
                quote! {
                    pub fn #decode_name(
                        &self,
                        output: fluentbase_sdk::Bytes
                    ) -> (#(#return_types,)*) {
                        #return_struct::decode(&output)
                            .expect("failed to decode result")
                            .0
                    }
                }
            }
        };

        quote! {
            #encode_fn
            #decode_fn
        }
    }

    /// Generates the main implementation method for contract calls
    fn generate_implementation(&self) -> TokenStream2 {
        let fn_name = &self.method.sig.ident;
        let encode_name = format_ident!("encode_{}", fn_name);
        let decode_name = format_ident!("decode_{}", fn_name);

        let params = self.abi.inputs.iter().map(|param| {
            let arg_info = param
                .fn_arg
                .as_ref()
                .expect("Parameter must have function argument info");
            let name = &arg_info.name;
            let name = format_ident!("{}", name);
            let ty = &arg_info.ty;
            quote! { #name: #ty }
        });

        let param_names = self.abi.inputs.iter().map(|param| {
            let name = &param
                .fn_arg
                .as_ref()
                .expect("Parameter must have function argument info")
                .name;

            format_ident!("{}", name)
        });

        let return_type = match self.abi.outputs.len() {
            0 => quote! { () },
            1 => {
                let ty = &self.abi.outputs[0].fn_arg.as_ref().unwrap().ty;
                quote! { #ty }
            }
            _ => {
                let types = self
                    .abi
                    .outputs
                    .iter()
                    .map(|param| &param.fn_arg.as_ref().unwrap().ty);
                quote! { (#(#types,)*) }
            }
        };

        quote! {
            pub fn #fn_name(
                &mut self,
                contract_address: fluentbase_sdk::Address,
                value: fluentbase_sdk::U256,
                gas_limit: u64,
                #(#params,)*
            ) -> #return_type {
                use fluentbase_sdk::TxContextReader;

                let input = self.#encode_name(#(#param_names,)*);
                {
                    let context = self.sdk.context();

                    if context.tx_value() < value {
                        ::core::panic!("Insufficient funds for transaction"
                        );
                    }
                    if context.tx_gas_limit() < gas_limit {
                        ::core::panic!("Insufficient gas limit for transaction");

                    }
                }

                let (output, exit_code) = self.sdk.call(
                    contract_address,
                    value,
                    &input,
                    gas_limit
                );

                if exit_code != 0 {
                    ::core::panic!("Contract call failed with exit code");
                }

                self.#decode_name(output)
            }
        }
    }
}

/// Generator for Router API client code
pub struct ClientGenerator {
    /// Router mode configuration
    mode: Mode,
    /// Original trait AST
    trait_ast: syn::ItemTrait,
}

impl Parse for ClientGenerator {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let mode = if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("client")) {
            attr.parse_args()?
        } else {
            Mode::default()
        };

        let trait_ast = input.parse()?;
        Ok(Self { mode, trait_ast })
    }
}

impl ClientGenerator {
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }
    /// Generates the complete client implementation
    fn generate_client(&self) -> Result<TokenStream2> {
        let trait_name = &self.trait_ast.ident;
        let client_name = format_ident!("{}Client", trait_name);

        let methods: Result<Vec<ClientMethod>> = self
            .trait_ast
            .items
            .iter()
            .filter_map(|item| {
                if let TraitItem::Fn(method) = item {
                    Some(ClientMethod::new(method, self.mode))
                } else {
                    None
                }
            })
            .collect();

        let methods = methods?;

        let codecs = methods.iter().map(|m| m.generate_codecs());
        let helpers = methods.iter().map(|m| m.generate_helpers());
        let implementations = methods.iter().map(|m| m.generate_implementation());

        Ok(quote! {
            #[derive(Debug)]
            pub struct #client_name<SDK> {
                pub sdk: SDK,
            }

            // Codec implementations
            #(#codecs)*

            // Helper methods implementation
            impl<SDK: fluentbase_sdk::SharedAPI> #client_name<SDK> {
                pub fn new(sdk: SDK) -> Self {
                    Self { sdk }
                }

                #(#helpers)*
            }

            // Contract calling methods implementation
            impl<SDK: fluentbase_sdk::SharedAPI> #client_name<SDK> {
                #(#implementations)*
            }
        })
    }
}

impl ToTokens for ClientGenerator {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        match self.generate_client() {
            Ok(implementation) => tokens.extend(implementation),
            Err(e) => tokens.extend(Error::to_compile_error(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_client_method_generation() {
        let method: TraitItemFn = parse_quote! {
            #[function_id("testMethod(uint64)")]
            fn test_method(&mut self, value: u64) -> String;
        };

        let client_method = ClientMethod::new(&method, Mode::default());
        assert!(client_method.is_ok());
    }

    #[test]
    fn test_client_trait_generator() {
        let input = quote! {
            trait TestAPI {
                #[function_id("testMethod(uint64)")]
                fn test_method(&mut self, value: u64) -> String;
            }
        };

        let generator: ClientGenerator = syn::parse2(input).unwrap();
        let output = generator.generate_client();
        assert!(output.is_ok());
    }
}

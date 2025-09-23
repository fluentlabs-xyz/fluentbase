use crate::{
    attr::mode::Mode,
    codec::CodecGenerator,
    method::{MethodCollector, ParsedMethod},
};
use darling::{ast::NestedMeta, FromMeta};
use proc_macro2::{Span, TokenStream as TokenStream2};
use proc_macro_error::{abort, abort_call_site, emit_error};
use quote::{format_ident, quote, ToTokens};
use syn::{spanned::Spanned, visit, Error, ImplItemFn, ItemImpl, Result};

/// Attributes for the constructor configuration.
#[derive(Debug, FromMeta, Default, Clone)]
pub struct ConstructorAttributes {
    /// The encoding mode (Solidity or Fluent)
    pub mode: Mode,
}

/// A constructor macro that generates deployment logic for smart contracts.
/// Handles parameter decoding and initialization during contract deployment.
#[derive(Debug)]
pub struct Constructor {
    /// Constructor configuration attributes
    attributes: ConstructorAttributes,
    /// The original implementation block
    impl_block: ItemImpl,
    /// The parsed constructor method
    constructor_method: ParsedMethod<ImplItemFn>,
}

/// Parses and validates a constructor from token streams.
pub fn process_constructor(attr: TokenStream2, input: TokenStream2) -> Result<Constructor> {
    let attributes = parse_attributes(attr)?;
    let impl_block = syn::parse2::<ItemImpl>(input)?;

    Constructor::new(attributes, impl_block)
}

/// Parses constructor attributes from a TokenStream.
fn parse_attributes(attr: TokenStream2) -> Result<ConstructorAttributes> {
    let meta = NestedMeta::parse_meta_list(attr)?;
    ConstructorAttributes::from_list(&meta)
        .map_err(|e| Error::new(Span::call_site(), e.to_string()))
}

impl Constructor {
    /// Creates a new Constructor instance by parsing the implementation block.
    pub fn new(attributes: ConstructorAttributes, impl_block: ItemImpl) -> Result<Self> {
        // Use the existing MethodCollector to find the constructor
        let is_trait_impl = impl_block.trait_.is_some();
        let mut collector =
            MethodCollector::<ImplItemFn>::new_for_impl(impl_block.span(), is_trait_impl);

        visit::visit_item_impl(&mut collector, &impl_block);

        // Validate we have exactly one constructor
        if collector.constructor.is_none() {
            abort!(
                impl_block.span(),
                "No constructor method found in implementation block";
                help = "Add a method named 'constructor' to initialize the contract";
                help = "Example: pub fn constructor(&mut self, initial_value: U256) {{ ... }}"
            );
        }

        // Check for any errors during collection
        if collector.has_errors() {
            for err in &collector.errors {
                emit_error!(err.span(), "{}", err.to_string());
            }
            abort_call_site!("Failed to process constructor due to parsing errors");
        }

        // Warn if regular methods were found (they'll be ignored)
        if !collector.methods.is_empty() {
            let method_names: Vec<String> = collector
                .methods
                .iter()
                .map(|m| m.parsed_signature().rust_name())
                .collect();

            emit_error!(
                impl_block.span(),
                "Found {} non-constructor methods that will be ignored: {}",
                collector.methods.len(),
                method_names.join(", ");
                note = "The #[constructor] macro only processes the 'constructor' method";
                help = "Use #[router] macro if you need to handle other methods"
            );
        }

        let constructor_method = collector
            .constructor
            .ok_or_else(|| Error::new(impl_block.span(), "Constructor method not found"))?;

        Ok(Self {
            attributes,
            impl_block,
            constructor_method,
        })
    }

    /// Returns the constructor attributes.
    pub fn attributes(&self) -> &ConstructorAttributes {
        &self.attributes
    }

    /// Returns the implementation block.
    pub fn impl_block(&self) -> &ItemImpl {
        &self.impl_block
    }

    /// Returns the constructor method.
    pub fn constructor_method(&self) -> &ParsedMethod<ImplItemFn> {
        &self.constructor_method
    }

    /// Generates the complete constructor implementation with all artifacts.
    pub fn generate(&self) -> Result<TokenStream2> {
        // Generate codec for constructor parameters
        let constructor_codec = self.generate_constructor_codec()?;

        // Generate deploy method
        let deploy_method = self.generate_deploy_method()?;

        // Keep the original impl block
        let impl_block = &self.impl_block;

        // Build the complete output
        Ok(quote! {
            #impl_block

            #constructor_codec

            #deploy_method
        })
    }

    /// Generates codec implementation for constructor parameters.
    fn generate_constructor_codec(&self) -> Result<TokenStream2> {
        CodecGenerator::new(&self.constructor_method, &self.attributes.mode)
            .generate()
            .map_err(|e| {
                Error::new(
                    self.constructor_method.parsed_signature().span(),
                    e.to_string(),
                )
            })
    }

    /// Generates the deploy method implementation.
    fn generate_deploy_method(&self) -> Result<TokenStream2> {
        let target_type = &self.impl_block.self_ty;
        let generic_params = &self.impl_block.generics;

        let constructor_call = self.generate_constructor_call()?;

        Ok(quote! {
            impl #generic_params #target_type {
                /// Deploy entry point for contract initialization.
                /// This method is called once during contract deployment.
                pub fn deploy(&mut self) {
                    #constructor_call
                }
            }
        })
    }

    /// Generates the constructor call logic with parameter decoding.
    fn generate_constructor_call(&self) -> Result<TokenStream2> {
        let fn_name = format_ident!("constructor");
        let params = self.constructor_method.parsed_signature().parameters();
        let param_count = params.len();

        let call_struct = format_ident!("ConstructorCall");

        // Generate parameter decoding based on parameter count
        let (param_handling, fn_call) = match param_count {
            0 => {
                // No parameters - simple call
                (quote! {}, quote! { self.#fn_name() })
            }
            1 => {
                // Single parameter
                let param_handling = quote! {
                    let param0 = match #call_struct::decode(&&call_data[..]) {
                        Ok(decoded) => decoded.0.0,
                        Err(err) => {
                            panic!("Failed to decode constructor parameter: {:?}", err);
                        }
                    };
                };
                let fn_call = quote! { self.#fn_name(param0) };
                (param_handling, fn_call)
            }
            _ => {
                // Multiple parameters
                let param_names = (0..param_count)
                    .map(|i| format_ident!("param{}", i))
                    .collect::<Vec<_>>();
                let param_indices = (0..param_count).map(syn::Index::from).collect::<Vec<_>>();

                let param_handling = quote! {
                    let (#(#param_names),*) = match #call_struct::decode(&&call_data[..]) {
                        Ok(decoded) => (#(decoded.0.#param_indices),*),
                        Err(err) => {
                            panic!("Failed to decode constructor parameters: {:?}", err);
                        }
                    };
                };

                let fn_call = quote! { self.#fn_name(#(#param_names),*) };
                (param_handling, fn_call)
            }
        };

        // Generate complete constructor call with input reading
        Ok(quote! {
            // Read input data
            let input_length = self.sdk.input_size();

            if input_length > 0 {
                // Read constructor parameters if provided
                let mut call_data = ::fluentbase_sdk::alloc_slice(input_length as usize);
                self.sdk.read(&mut call_data, 0);

                // Decode parameters and call constructor
                #param_handling
                #fn_call;
            } else {
                // Call constructor without parameters
                #fn_call;
            }
        })
    }
}

impl ToTokens for Constructor {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        match self.generate() {
            Ok(generated) => tokens.extend(generated),
            Err(e) => tokens.extend(Error::to_compile_error(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use syn::parse_quote;

    #[test]
    fn test_constructor_no_params() {
        let impl_block: ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> MyContract<SDK> {
                pub fn constructor(&mut self) {
                    // Initialize without parameters
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };
        let constructor = process_constructor(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process constructor");

        let generated = constructor
            .generate()
            .expect("Failed to generate constructor code");

        let file = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("constructor_no_params", formatted);
    }

    #[test]
    fn test_constructor_with_params() {
        let impl_block: ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> Token<SDK> {
                pub fn constructor(&mut self, owner: Address, initial_supply: U256) {
                    // Initialize with parameters
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };
        let constructor = process_constructor(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process constructor");

        let generated = constructor
            .generate()
            .expect("Failed to generate constructor code");

        let file = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("constructor_with_params", formatted);
    }
}

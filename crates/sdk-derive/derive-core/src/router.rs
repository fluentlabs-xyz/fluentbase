use crate::{
    attr::mode::Mode,
    codec::CodecGenerator,
    method::{MethodCollector, ParsedMethod},
};
use convert_case::{Case, Casing};
use darling::{ast::NestedMeta, FromMeta};
use proc_macro2::{Span, TokenStream as TokenStream2};
use proc_macro_error::{abort, abort_call_site, emit_error};
use quote::{format_ident, quote, ToTokens};
use syn::{spanned::Spanned, visit, Error, Ident, ImplItemFn, ItemImpl, Result};

/// Attributes for the router configuration.
#[derive(Debug, FromMeta, Default, Clone)]
pub struct RouterAttributes {
    /// The router mode (Solidity or Fluent)
    pub mode: Mode,
}

/// A router that handles method dispatch based on input selectors.
/// Supports both Solidity-compatible and Fluent API modes.
#[derive(Debug)]
pub struct Router {
    /// Router configuration attributes
    attributes: RouterAttributes,
    /// The original implementation block
    impl_block: ItemImpl,
    /// Collection of available method routes
    routes: Vec<ParsedMethod<ImplItemFn>>,
    /// Constructor method if defined
    constructor: Option<ParsedMethod<ImplItemFn>>,
    /// Indicates whether this is a trait implementation
    is_trait_impl: bool,
}

/// Parses and validates a router from token streams.
pub fn process_router(attr: TokenStream2, input: TokenStream2) -> Result<Router> {
    let attributes = parse_attributes(attr)?;

    let impl_block = syn::parse2::<ItemImpl>(input)?;

    let router = Router::new(attributes, impl_block)?;

    Ok(router)
}

/// Parses router attributes from a TokenStream.
fn parse_attributes(attr: TokenStream2) -> Result<RouterAttributes> {
    let meta = NestedMeta::parse_meta_list(attr)?;
    RouterAttributes::from_list(&meta).map_err(|e| Error::new(Span::call_site(), e.to_string()))
}

impl Router {
    /// Creates a new Router instance by parsing the implementation block.
    pub fn new(attributes: RouterAttributes, impl_block: ItemImpl) -> Result<Self> {
        let is_trait_impl = impl_block.trait_.is_some();

        let mut collector =
            MethodCollector::<ImplItemFn>::new_for_impl(impl_block.span(), is_trait_impl);
        visit::visit_item_impl(&mut collector, &impl_block);

        if collector.methods.is_empty() && collector.constructor.is_none() {
            abort!(
                collector.span,
                "Router has no methods or constructor. Make sure your implementation contains at least one public method or a constructor.";
                help = "Check that methods are public (pub fn) for regular implementations";
                help = if is_trait_impl {
                    "For trait implementations, make sure the trait contains method declarations"
                } else {
                    "Consider marking your methods as public: pub fn method_name(...)"
                }
            );
        }

        if collector.has_errors() {
            for err in &collector.errors {
                emit_error!(err.span(), "{}", err.to_string());
            }

            abort_call_site!(
                "Failed to process router implementation due to method parsing errors"
            );
        }

        if let Err(collision_error) = collector.validate_selectors() {
            abort!(
                collision_error.span(),
                "{}",
                collision_error.to_string();
                help = "Function selectors must be unique across all methods";
                help = "You can use custom selectors with #[function_id(\"custom_signature\")]";
                help = "Or rename your methods to have different signatures"
            );
        }

        Ok(Self {
            attributes,
            impl_block,
            routes: collector.methods,
            constructor: collector.constructor,
            is_trait_impl,
        })
    }

    /// Returns the router attributes.
    pub fn attributes(&self) -> &RouterAttributes {
        &self.attributes
    }

    /// Returns the implementation block.
    pub fn impl_block(&self) -> &ItemImpl {
        &self.impl_block
    }

    /// Returns the list of parsed routes.
    pub fn routes(&self) -> &[ParsedMethod<ImplItemFn>] {
        &self.routes
    }

    /// Returns all available method routes excluding fallback.
    pub fn available_methods(&self) -> Vec<&ParsedMethod<ImplItemFn>> {
        self.routes
            .iter()
            .filter(|route| route.parsed_signature().rust_name() != "fallback")
            .collect()
    }

    /// Checks if the router is based on a trait implementation.
    pub fn is_trait_impl(&self) -> bool {
        self.is_trait_impl
    }

    /// Checks if the router has a fallback handler.
    pub fn has_fallback(&self) -> bool {
        self.routes
            .iter()
            .any(|r| r.parsed_signature().is_fallback())
    }

    /// Checks if the router has a constructor.
    pub fn has_constructor(&self) -> bool {
        self.constructor.is_some()
    }

    /// Returns the constructor method if present.
    pub fn constructor(&self) -> Option<&ParsedMethod<ImplItemFn>> {
        self.constructor.as_ref()
    }

    /// Returns the trait name if this is a trait implementation, None otherwise.
    pub fn trait_name(&self) -> Option<String> {
        self.impl_block
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .map(|segment| segment.ident.to_string())
    }

    /// Returns the struct name from the implementation block.
    pub fn struct_name(&self) -> String {
        match &*self.impl_block.self_ty {
            syn::Type::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default(),
            _ => self.impl_block.self_ty.to_token_stream().to_string(),
        }
    }

    /// Returns the module name for the generated router code.
    pub fn module_name(&self) -> Ident {
        if let Some(trait_name) = self.trait_name() {
            format_ident!(
                "{}_{}",
                trait_name.to_case(Case::Snake),
                self.struct_name().to_case(Case::Snake)
            )
        } else {
            format_ident!("{}", self.struct_name().to_case(Case::Snake))
        }
    }

    /// Returns the contract name for artifact generation.
    pub fn contract_name(&self) -> String {
        if let Some(trait_name) = self.trait_name() {
            format!(
                "{}{}",
                trait_name.to_case(Case::Pascal),
                self.struct_name().to_case(Case::Pascal)
            )
        } else {
            self.struct_name().to_case(Case::Pascal)
        }
    }

    /// Generates the complete router implementation with all artifacts.
    pub fn generate(&self) -> Result<TokenStream2> {
        // Start with the implementation block
        let impl_block = self.impl_block();

        // Generate codec implementations
        let method_codecs = self.generate_codec_implementations()?;

        // Generate dispatch method
        let dispatch_method = self.generate_dispatch_method()?;

        // Generate deploy method if constructor exists
        let deploy_method = if self.has_constructor() {
            self.generate_deploy_method()?
        } else {
            quote! {}
        };

        // Build the base output
        let output = quote! {
            #[allow(unused_imports)]
            use ::fluentbase_sdk::derive::function_id;
            #impl_block

            #(#method_codecs)*

            #dispatch_method
            #deploy_method
        };

        Ok(output)
    }

    /// Generates codec implementations for all available methods.
    fn generate_codec_implementations(&self) -> Result<Vec<TokenStream2>> {
        let mut codecs = self
            .available_methods()
            .iter()
            .map(|route| {
                CodecGenerator::new(*route, &self.attributes().mode)
                    .generate()
                    .map_err(|e| Error::new(route.parsed_signature().span(), e.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;

        // Add codec for constructor if present
        if let Some(constructor) = &self.constructor {
            let constructor_codec = CodecGenerator::new(constructor, &self.attributes().mode)
                .generate()
                .map_err(|e| Error::new(constructor.parsed_signature().span(), e.to_string()))?;
            codecs.push(constructor_codec);
        }

        Ok(codecs)
    }

    /// Generates the main dispatch method implementation.
    fn generate_dispatch_method(&self) -> Result<TokenStream2> {
        let target_type = &self.impl_block.self_ty;
        let generic_params = &self.impl_block.generics;

        let input_validation = self.generate_input_validation();
        let method_arms = self.generate_method_arms()?;

        Ok(quote! {
            impl #generic_params #target_type {
                pub fn main(&mut self) {
                    let input_length = self.sdk.input_size();
                    #input_validation

                    let mut call_data = ::fluentbase_sdk::alloc_slice(input_length as usize);
                    self.sdk.read(&mut call_data, 0);

                    let (selector, params) = call_data.split_at(4);

                    match [selector[0], selector[1], selector[2], selector[3]] {
                        #method_arms
                    }
                }
            }
        })
    }

    /// Generates the deploy method implementation for constructor.
    // В методе generate_deploy_method(), исправим обработку параметров:

    fn generate_deploy_method(&self) -> Result<TokenStream2> {
        let Some(constructor) = &self.constructor else {
            return Ok(quote! {});
        };

        let target_type = &self.impl_block.self_ty;
        let generic_params = &self.impl_block.generics;

        let fn_name = format_ident!("constructor");
        let params = constructor.parsed_signature().parameters();
        let param_count = params.len();

        // Generate struct name for codec
        let call_struct = format_ident!("ConstructorCall");

        // Generate parameter handling based on count
        let (param_handling, fn_call) = match param_count {
            0 => (quote! {}, quote! { self.#fn_name() }),
            1 => (
                quote! {
                    let param0 = match #call_struct::decode(&call_data) {
                        Ok(decoded) => decoded.0.0,
                        Err(err) => {
                            panic!("Failed to decode constructor parameters: {:?}", err);
                        }
                    };
                },
                quote! { self.#fn_name(param0) },
            ),
            _ => {
                let param_names = (0..param_count)
                    .map(|i| format_ident!("param{}", i))
                    .collect::<Vec<_>>();

                let param_indices = (0..param_count).map(syn::Index::from).collect::<Vec<_>>();

                (
                    quote! {
                        let decoded = match #call_struct::decode(&call_data) {
                            Ok(decoded) => decoded,
                            Err(err) => {
                                panic!("Failed to decode constructor parameters: {:?}", err);
                            }
                        };
                        #(let #param_names = decoded.0.#param_indices;)*
                    },
                    quote! { self.#fn_name(#(#param_names),*) },
                )
            }
        };

        Ok(quote! {
            impl #generic_params #target_type {
                pub fn deploy(&mut self) {
                    let input_length = self.sdk.input_size();

                    if input_length > 0 {
                        let mut call_data = ::fluentbase_sdk::alloc_slice(input_length as usize);
                        self.sdk.read(&mut call_data, 0);

                        #param_handling
                        #fn_call;
                    } else {
                        // Constructor with no parameters
                        #fn_call;
                    }
                }
            }
        })
    }
    /// Generates input validation logic.
    fn generate_input_validation(&self) -> TokenStream2 {
        if self.has_fallback() {
            quote! {
                if input_length < 4 {
                    self.fallback();
                    return;
                }
            }
        } else {
            quote! {
                if input_length < 4 {
                    panic!("insufficient input length for method selector");
                }
            }
        }
    }

    /// Generates method dispatch match arms.
    fn generate_method_arms(&self) -> Result<TokenStream2> {
        let mut arms = Vec::new();

        for route in self.available_methods() {
            let arm = self.generate_method_arm(route)?;
            arms.push(arm);
        }

        let fallback_arm = self.generate_fallback_arm();

        Ok(quote! {
            #(#arms),*,
            #fallback_arm
        })
    }

    /// Generates a single method match arm.
    fn generate_method_arm(&self, route: &ParsedMethod<ImplItemFn>) -> Result<TokenStream2> {
        let function_id = route.function_id();
        let fn_name_str = route.parsed_signature().rust_name();
        let fn_name = format_ident!("{}", fn_name_str);

        let call_struct = format_ident!("{}Call", fn_name_str.to_case(Case::Pascal));
        let return_struct = format_ident!("{}Return", fn_name_str.to_case(Case::Pascal));

        let params = route.parsed_signature().parameters();
        let param_count = params.len();
        let return_type_count = route.parsed_signature().return_type().len();

        // Generate parameter handling based on parameter count
        let param_handling = match param_count {
            0 => quote! {},
            1 => {
                quote! {
                    let param0 = match #call_struct::decode(&params) {
                        Ok(decoded) => decoded.0.0,
                        Err(err) => {
                            panic!("Failed to decode parameters: {:?}", err);
                        }
                    };
                }
            }
            _ => {
                let param_names = (0..param_count)
                    .map(|i| format_ident!("param{}", i))
                    .collect::<Vec<_>>();

                let param_indices = (0..param_count).map(syn::Index::from).collect::<Vec<_>>();

                quote! {
                    let (#(#param_names),*) = match #call_struct::decode(&params) {
                        Ok(decoded) => (#(decoded.0.#param_indices),*),
                        Err(err) => {
                            panic!("Failed to decode parameters: {:?}", err);
                        }
                    };
                }
            }
        };

        // Generate function call based on parameter count
        let fn_call = match param_count {
            0 => quote! { self.#fn_name() },
            1 => quote! { self.#fn_name(param0) },
            _ => {
                let param_names = (0..param_count)
                    .map(|i| format_ident!("param{}", i))
                    .collect::<Vec<_>>();

                quote! { self.#fn_name(#(#param_names),*) }
            }
        };

        // Generate result handling based on return type count
        let result_handling = match return_type_count {
            0 => quote! {
                let _ = #fn_call;
                let encoded_output = [0u8; 0];
                self.sdk.write(&encoded_output);
            },
            1 => quote! {
                let output = #fn_call;
                let encoded_output = #return_struct::new((output,)).encode();
                self.sdk.write(&encoded_output);
            },
            _ => quote! {
                let output = #fn_call;
                let encoded_output = #return_struct::new(output).encode();
                self.sdk.write(&encoded_output);
            },
        };

        Ok(quote! {
            [#(#function_id),*] => {
                #param_handling
                #result_handling
            }
        })
    }

    /// Generates the fallback handler match arm.
    fn generate_fallback_arm(&self) -> TokenStream2 {
        if self.has_fallback() {
            quote! {
                _ => {
                    self.fallback();
                }
            }
        } else {
            quote! {
                _ => panic!("unsupported method selector"),
            }
        }
    }
}

impl ToTokens for Router {
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
    use prettyplease;
    use quote::quote;
    use syn::{parse_file, parse_quote};

    #[test]
    fn test_trait_router_generation() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> RouterAPI for App<SDK> {
                fn greeting(&self, message: String, owner: Address, amount: U256) -> String {
                    message
                }

                #[function_id("customGreeting(string)")]
                fn custom_greeting(&self, message: String) -> String {
                    message
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };

        let router = process_router(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process router");

        let generated = router.generate().expect("Failed to generate router code");

        let file = parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("trait_router_generation", formatted);
    }
}

use crate::{
    abi::structs::StructResolver,
    attr::{mode::Mode, state_mutability::StateMutabilityExt, STATE_MUTABILITY_ATTR},
    codec::CodecGenerator,
    method::{combine_errors, MethodCollector, ParsedMethod},
};
use convert_case::{Case, Casing};
use darling::{ast::NestedMeta, FromMeta};
use proc_macro2::{Span, TokenStream as TokenStream2};
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
///
/// Struct parameters are expanded from the sources of the crate being compiled, so the selectors
/// baked into the dispatch table match the ones callers derive from the published ABI.
pub fn process_router(attr: TokenStream2, input: TokenStream2) -> Result<Router> {
    process_router_with_structs(attr, input, &StructResolver::crate_sources())
}

/// Parses and validates a router, resolving struct parameters through the given resolver.
///
/// Build tooling uses this to reuse the registry it has already parsed, which is what keeps the
/// artifacts it generates in step with the router the macro compiles.
pub fn process_router_with_structs(
    attr: TokenStream2,
    input: TokenStream2,
    resolver: &StructResolver,
) -> Result<Router> {
    let attributes = parse_attributes(attr)?;

    let impl_block = syn::parse2::<ItemImpl>(input)?;

    let router = Router::new(attributes, impl_block, resolver)?;

    Ok(router)
}

/// Parses router attributes from a TokenStream.
fn parse_attributes(attr: TokenStream2) -> Result<RouterAttributes> {
    let meta = NestedMeta::parse_meta_list(attr)?;
    RouterAttributes::from_list(&meta).map_err(|e| Error::new(Span::call_site(), e.to_string()))
}

impl Router {
    /// Creates a new Router instance by parsing the implementation block.
    pub fn new(
        attributes: RouterAttributes,
        impl_block: ItemImpl,
        resolver: &StructResolver,
    ) -> Result<Self> {
        let is_trait_impl = impl_block.trait_.is_some();

        let mut collector =
            MethodCollector::<ImplItemFn>::new_for_impl(impl_block.span(), is_trait_impl, resolver);
        visit::visit_item_impl(&mut collector, &impl_block);

        // Errors are returned rather than reported through `proc_macro_error`, because the build
        // tooling drives this same code outside of a proc-macro expansion. They are also reported
        // before the emptiness check, since a method that failed to parse was never collected.
        if let Some(error) = combine_errors(std::mem::take(&mut collector.errors)) {
            return Err(error);
        }

        if collector.methods.is_empty() && collector.constructor.is_none() {
            let help = if is_trait_impl {
                "For trait implementations, make sure the trait contains method declarations"
            } else {
                "Consider marking your methods as public: pub fn method_name(...)"
            };

            return Err(Error::new(
                collector.span,
                format!(
                    "Router has no methods or constructor. Make sure your implementation contains \
                     at least one public method or a constructor.\n\
                     help: Check that methods are public (pub fn) for regular implementations\n\
                     help: {help}"
                ),
            ));
        }

        if let Err(collision_error) = collector.validate_selectors() {
            return Err(Error::new(
                collision_error.span(),
                format!(
                    "{collision_error}\n\
                     help: Function selectors must be unique across all methods\n\
                     help: You can use custom selectors with #[function_id(\"custom_signature\")]\n\
                     help: Or rename your methods to have different signatures"
                ),
            ));
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
        let mut clean_impl_block = self.impl_block.clone();

        for item in &mut clean_impl_block.items {
            if let syn::ImplItem::Fn(method) = item {
                method.attrs.retain(|attr| {
                    !attr.path().is_ident("function_id")
                        && !attr.path().is_ident(STATE_MUTABILITY_ATTR)
                });
            }
        }

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
            #clean_impl_block

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

                    let mut call_data = alloc::vec![0u8; input_length as usize];
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
    fn generate_deploy_method(&self) -> Result<TokenStream2> {
        let Some(constructor) = &self.constructor else {
            return Ok(quote! {});
        };

        let target_type = &self.impl_block.self_ty;
        let generic_params = &self.impl_block.generics;

        let deploy_body = constructor.generate_deploy_body();

        Ok(quote! {
            impl #generic_params #target_type {
                pub fn deploy(&mut self) {
                    #deploy_body
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

        let value_guard = if route.state_mutability().allows_value() {
            quote! {}
        } else {
            quote! {
                use fluentbase_sdk::ContextReader as _;
                if !self.sdk.context().contract_value().is_zero() {
                    panic!("nonpayable method cannot receive value");
                }
            }
        };

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
                #value_guard
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
    use std::fs;
    use syn::{parse_file, parse_quote};
    use tempfile::TempDir;

    /// A resolver over a crate consisting of the given root source
    fn resolver_for(source: &str) -> (TempDir, StructResolver) {
        let temp_dir = TempDir::new().unwrap();
        let entry_file = temp_dir.path().join("lib.rs");
        fs::write(&entry_file, source).unwrap();
        let registry = crate::abi::structs::StructRegistry::parse_crate(&entry_file).unwrap();
        (temp_dir, StructResolver::registry(registry))
    }

    /// A struct the crate does not define cannot be hashed into a selector, so it fails the build
    /// instead of silently collapsing into an empty tuple
    #[test]
    fn test_unresolved_struct_parameter_is_rejected() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn create_user(&mut self, user: User) -> bool {
                    true
                }
            }
        };

        let (_temp, resolver) = resolver_for("#[derive(Codec)] pub struct Other { pub v: U256 }");

        let error = process_router_with_structs(
            quote! { mode = "solidity" },
            impl_block.into_token_stream(),
            &resolver,
        )
        .expect_err("an unresolved struct parameter should fail")
        .to_string();

        assert!(
            error.contains("no `#[derive(Codec)]` definition"),
            "unexpected error: {error}"
        );
    }

    /// A struct whose name matches several modules is rejected rather than resolved arbitrarily
    #[test]
    fn test_ambiguous_struct_parameter_is_rejected() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn set(&mut self, config: Config) -> bool {
                    true
                }
            }
        };

        let (_temp, resolver) = resolver_for(
            r#"
mod a {
    #[derive(Codec)]
    pub struct Config { pub value: U256 }
}

mod b {
    #[derive(Codec)]
    pub struct Config { pub owner: Address }
}
"#,
        );

        let error = process_router_with_structs(
            quote! { mode = "solidity" },
            impl_block.into_token_stream(),
            &resolver,
        )
        .expect_err("an ambiguous struct parameter should fail")
        .to_string();

        assert!(error.contains("ambiguous"), "unexpected error: {error}");
    }

    /// Struct components are expanded before the selector is hashed
    #[test]
    fn test_struct_parameter_selector_uses_resolved_components() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn set_a(&mut self, config: Config) -> bool {
                    true
                }
            }
        };

        let (_temp, resolver) = resolver_for(
            "#[derive(Codec)] pub struct Config { pub value: U256, pub enabled: bool }",
        );

        let router = process_router_with_structs(
            quote! { mode = "solidity" },
            impl_block.into_token_stream(),
            &resolver,
        )
        .expect("Failed to process router");

        let method = router.available_methods()[0];
        assert_eq!(method.signature(), "setA((uint256,bool))");
        // keccak256("setA((uint256,bool))")[..4]
        assert_eq!(method.function_id(), [0xb6, 0xea, 0x7d, 0x04]);
    }

    /// A pinned selector still works when the struct cannot be resolved here at all
    #[test]
    fn test_function_id_pins_the_selector_of_an_unresolved_struct() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                #[function_id("setA((uint256,bool))")]
                pub fn set_a(&mut self, config: external_crate::Config) -> bool {
                    true
                }
            }
        };

        let (_temp, resolver) = resolver_for("");

        let router = process_router_with_structs(
            quote! { mode = "solidity" },
            impl_block.into_token_stream(),
            &resolver,
        )
        .expect("a pinned selector should not need the struct definition");

        let method = router.available_methods()[0];
        assert_eq!(method.signature(), "setA((uint256,bool))");
        assert_eq!(method.function_id(), [0xb6, 0xea, 0x7d, 0x04]);
    }

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

    #[test]
    fn test_constructor_no_params() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn constructor(&mut self) {
                    // Initialize without parameters
                }

                pub fn get_value(&self) -> u32 {
                    42
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };

        let router = process_router(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process router");

        let generated = router.generate().expect("Failed to generate router code");

        let file = parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("constructor_no_params", formatted);
    }

    #[test]
    fn test_constructor_single_param() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn constructor(&mut self, initial_value: U256) {
                    // Initialize with single parameter
                }

                pub fn get_value(&self) -> U256 {
                    U256::from(0)
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };

        let router = process_router(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process router");

        let generated = router.generate().expect("Failed to generate router code");

        let file = parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("constructor_single_param", formatted);
    }

    #[test]
    fn test_constructor_two_params() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                pub fn constructor(&mut self, owner: Address, initial_supply: U256) {
                    // Initialize with two parameters
                }

                pub fn balance_of(&self, account: Address) -> U256 {
                    U256::from(0)
                }

                pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
                    true
                }
            }
        };

        let attr_tokens = quote! { mode = "solidity" };

        let router = process_router(attr_tokens, impl_block.into_token_stream())
            .expect("Failed to process router");

        let generated = router.generate().expect("Failed to generate router code");

        let file = parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("constructor_two_params", formatted);
    }

    #[test]
    fn test_nonpayable_routes_and_constructor_reject_value() {
        let impl_block: syn::ItemImpl = parse_quote! {
            impl<SDK: SharedAPI> App<SDK> {
                #[state_mutability("nonpayable")]
                pub fn constructor(&mut self) {}

                #[state_mutability("nonpayable")]
                pub fn mutate(&mut self) {}

                #[state_mutability("payable")]
                pub fn deposit(&mut self) {}
            }
        };

        let router = process_router(quote! { mode = "solidity" }, impl_block.into_token_stream())
            .expect("Failed to process router");
        let generated = router
            .generate()
            .expect("Failed to generate router code")
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();

        assert_eq!(
            generated
                .matches("if!self.sdk.context().contract_value().is_zero()")
                .count(),
            2
        );
    }
}

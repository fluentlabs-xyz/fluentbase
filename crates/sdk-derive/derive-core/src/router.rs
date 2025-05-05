use crate::{artifacts::generate_sol_interface, mode::RouterMode, route::Route};
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use std::{fs, path::Path};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Error,
    ImplItem,
    ItemImpl,
    Result,
    Type,
};

/// A router that handles method dispatch based on input selectors.
/// Supports both Solidity-compatible and Fluent API modes.
pub struct Router {
    /// The routing mode (Solidity or Fluent)
    pub mode: RouterMode,
    /// Path to the artifacts directory
    pub artifacts_path: Option<String>,
    /// The original implementation AST
    implementation: ItemImpl,
    /// Collection of available method routes
    method_routes: Vec<Route>,
    /// Indicates whether a fallback handler is present
    has_fallback_handler: bool,
}

impl Router {
    /// Generates the router implementation including method dispatch logic.
    fn generate_router(&self) -> TokenStream2 {
        let target_type = &self.implementation.self_ty;
        let type_params = &self.implementation.generics;

        let available_routes = self.get_available_methods();
        let method_implementations = self.generate_method_implementations(&available_routes);
        let fallback_handler = self.generate_fallback_handler();
        let dispatch_arms =
            self.generate_dispatch_logic(&method_implementations, &fallback_handler);
        let input_validation = self.generate_input_validation();

        self.generate_router_implementation(
            target_type,
            type_params,
            &dispatch_arms,
            &input_validation,
        )
    }

    /// Returns all available method routes excluding fallback
    fn get_available_methods(&self) -> Vec<&Route> {
        self.method_routes
            .iter()
            .filter(|route| route.fn_name != "fallback")
            .collect()
    }

    /// Generates token streams for method implementations
    fn generate_method_implementations(&self, routes: &[&Route]) -> Vec<TokenStream2> {
        routes.iter().map(|route| route.to_token_stream()).collect()
    }

    /// Generates the fallback handling logic
    fn generate_fallback_handler(&self) -> TokenStream2 {
        if self.has_fallback_handler {
            quote! {
                _ => {
                    self.fallback();
                },
            }
        } else {
            quote! {
                _ => panic!("unsupported method selector"),
            }
        }
    }

    /// Generates method dispatch match arms
    fn generate_dispatch_logic(
        &self,
        method_implementations: &[TokenStream2],
        fallback_handler: &TokenStream2,
    ) -> TokenStream2 {
        if method_implementations.is_empty() {
            quote! {
                _ => panic!("no methods available for routing"),
            }
        } else {
            quote! {
                #(#method_implementations),*,
                #fallback_handler
            }
        }
    }

    /// Generates input validation logic
    fn generate_input_validation(&self) -> TokenStream2 {
        if self.has_fallback_handler {
            quote! {
                if input_length < 4 {
                    self.fallback();
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

    /// Generates the final router implementation
    fn generate_router_implementation(
        &self,
        target_type: &Box<Type>,
        type_params: &syn::Generics,
        dispatch_arms: &TokenStream2,
        input_validation: &TokenStream2,
    ) -> TokenStream2 {
        quote! {
            impl #type_params #target_type {
                pub fn main(&mut self) {
                    let input_length = self.sdk.input_size();
                    #input_validation

                    let mut call_data = fluentbase_sdk::alloc_slice(input_length as usize);
                    self.sdk.read(&mut call_data, 0);

                    let (selector, params) = call_data.split_at(4);

                    match [selector[0], selector[1], selector[2], selector[3]] {
                        #dispatch_arms
                    }
                }
            }
        }
    }
}

impl Parse for Router {
    fn parse(input: ParseStream) -> Result<Self> {
        let implementation: ItemImpl = input.parse()?;
        let all_routes = parse_implementation_methods(&implementation)?;

        let available_routes = if implementation.trait_.is_some() {
            all_routes.clone()
        } else {
            all_routes
                .into_iter()
                .filter(|route| route.is_public && route.fn_name != "deploy")
                .collect()
        };

        let has_fallback_handler = available_routes
            .iter()
            .any(|method| method.fn_name == "fallback");

        Ok(Router {
            mode: RouterMode::Solidity, // default mode
            artifacts_path: None,
            implementation,
            method_routes: available_routes,
            has_fallback_handler,
        })
    }
}

impl ToTokens for Router {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let implementation = &self.implementation;
        let available_routes = self.get_available_methods();

        let method_codecs = available_routes
            .iter()
            .map(|route| route.generate_codec_impl(&self.mode))
            .collect::<Vec<_>>();

        let router = self.generate_router();

        output.extend(quote! {
            #implementation
            #(#method_codecs)*
            #router
        });
    }
}

impl Router {
    // Generate Solidity interface artifacts
    pub fn generate_artifacts(&self, artifacts_path: &str) {
        // Extract contract name from implementation
        let contract_name = if let Some((_, trait_path, _)) = &self.implementation.trait_ {
            trait_path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_else(|| "Contract".to_string())
        } else {
            // If no trait, use the type name
            let type_name = &self.implementation.self_ty;
            format!("{}", quote!(#type_name))
        };

        // Create functions ABI
        let functions_abi = self
            .method_routes
            .iter()
            .filter(|route| route.fn_name != "fallback")
            .map(|route| route.to_abi_json())
            .collect::<Vec<_>>();

        // Create directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(artifacts_path) {
            eprintln!("Failed to create artifacts directory: {}", e);
            return;
        }

        // Generate Solidity interface
        if let Ok(interface) = generate_sol_interface(&contract_name, &functions_abi) {
            // Write interface file
            let interface_path = Path::new(artifacts_path).join(format!("I{}.sol", contract_name));
            if let Err(e) = fs::write(&interface_path, interface) {
                eprintln!("Failed to write interface file: {}", e);
            }
        }

        // Generate JSON ABI
        let abi_json = serde_json::to_string_pretty(&functions_abi).unwrap_or_else(|e| {
            eprintln!("Failed to serialize ABI JSON: {}", e);
            "[]".to_string()
        });

        // Write ABI JSON file
        let abi_path = Path::new(artifacts_path).join(format!("{}.json", contract_name));
        if let Err(e) = fs::write(&abi_path, abi_json) {
            eprintln!("Failed to write ABI JSON file: {}", e);
        }
    }
}

/// Parses methods from an implementation and converts them to Routes.
///
/// # Arguments
/// * `implementation` - The implementation AST to parse
///
/// # Returns
/// * `Result<Vec<Route>>` - Collection of parsed routes or error
fn parse_implementation_methods(implementation: &ItemImpl) -> Result<Vec<Route>> {
    let mut routes = Vec::new();
    let mut parse_errors = Vec::new();

    for item in &implementation.items {
        if let ImplItem::Fn(method) = item {
            match syn::parse2::<Route>(quote! { #method }) {
                Ok(route) => routes.push(route),
                Err(error) => parse_errors.push(Error::new(
                    method.span(),
                    format!("Failed to parse method '{}': {}", &method.sig.ident, error),
                )),
            }
        }
    }

    if parse_errors.is_empty() {
        Ok(routes)
    } else {
        Err(parse_errors
            .into_iter()
            .reduce(|mut combined, error| {
                combined.combine(error);
                combined
            })
            .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use prettyplease;
    use syn::parse_quote;

    #[test]
    fn test_generate_router() {
        let implementation: ItemImpl = parse_quote! {
            impl TestContract {
                pub fn first_method(&mut self, value: u32) -> u32 {
                    // Implementation details
                    value * 2
                }

                pub fn second_method(&mut self, a: String, b: bool) -> (String, bool) {
                    // Implementation details
                    (a, b)
                }
            }
        };

        let router = syn::parse2::<Router>(quote! { #implementation }).unwrap();

        // Set the router mode explicitly if needed
        // router.mode = RouterMode::Solidity;

        let generated_tokens = router.to_token_stream();
        let file = syn::parse_file(&generated_tokens.to_string()).unwrap();

        let formatted_code = prettyplease::unparse(&file);

        assert_snapshot!("generate_router", formatted_code);
    }
}

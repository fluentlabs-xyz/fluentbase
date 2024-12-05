use crate::{mode::Mode, route::Route};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use serde_json::Value;
use std::{env, fs, path::Path};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Error,
    ImplItem,
    ItemImpl,
    Result,
    Visibility,
};

/// A router that handles method dispatch based on input selectors.
/// Supports both trait implementations and regular implementations.
pub struct Router {
    /// The routing mode (Solidity or Fluent)
    pub mode: Mode,
    /// The original implementation AST
    item: ItemImpl,
    /// Collection of available method routes
    routes: Vec<Route>,
    /// Indicates whether a fallback handler is present
    has_fallback_handler: bool,
}

impl Router {
    /// Generates the router implementation including method dispatch logic.
    fn generate(&self) -> TokenStream2 {
        let impl_ty = &self.item.self_ty;
        let generic_params = &self.item.generics;

        let routes = self.routes();
        let dispatch_arms = self.generate_dispatch_logic(&routes);
        let input_validation = self.generate_input_validation();

        quote! {
            impl #generic_params #impl_ty {
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

    /// Generates method dispatch match arms
    fn generate_dispatch_logic(&self, routes: &[&Route]) -> TokenStream2 {
        let routes: Vec<TokenStream2> =
            routes.iter().map(|route| route.to_token_stream()).collect();

        let fallback = self.generate_fallback_handler();

        quote! {
            #(#routes),*,
            #fallback
        }
    }

    /// Generates input validation logic
    fn generate_input_validation(&self) -> TokenStream2 {
        if self.has_fallback_handler {
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

    /// Returns all available method routes excluding fallback and deploy
    fn routes(&self) -> Vec<&Route> {
        self.routes
            .iter()
            .filter(|route| {
                let method_name = route.method().sig.ident.to_string();
                method_name != "fallback" && method_name != "deploy"
            })
            .collect()
    }
}

fn parse_routes(items: &[ImplItem], is_trait_impl: bool) -> Result<Vec<Route>> {
    let mut routes = Vec::new();

    for item in items {
        if let ImplItem::Fn(method) = item {
            // Skip deploy function
            if method.sig.ident == "deploy" {
                continue;
            }

            // For regular implementations, only process public methods
            if !is_trait_impl && !matches!(method.vis, Visibility::Public(_)) {
                continue;
            }

            match Route::new(method.clone()) {
                Ok(route) => {
                    routes.push(route);
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }
    }

    Ok(routes)
}

fn check_fallback(routes: &[Route]) -> bool {
    routes
        .iter()
        .any(|route| route.method().sig.ident.to_string() == "fallback")
}

impl Parse for Router {
    fn parse(input: ParseStream) -> Result<Self> {
        let item: ItemImpl = input.parse()?;
        let is_trait_impl = item.trait_.is_some();

        if !is_trait_impl {
            // For regular implementations, check for at least one public method
            let has_public_methods = item.items.iter().any(|item| {
                if let ImplItem::Fn(method) = item {
                    matches!(method.vis, Visibility::Public(_))
                } else {
                    false
                }
            });

            if !has_public_methods {
                return Err(Error::new(
                    item.span(),
                    "regular implementation must have at least one public method for routing",
                ));
            }
        }

        let routes = parse_routes(&item.items, is_trait_impl)?;
        let has_fallback_handler = check_fallback(&routes);

        Ok(Router {
            mode: Mode::Solidity,
            item,
            routes,
            has_fallback_handler,
        })
    }
}

struct ContractArtifacts {
    base_name: String,
    abi_content: String,
    interface_content: String,
}

impl ContractArtifacts {
    /// Save artifacts to the specified directory
    fn save_to_dir(&self, dir: &Path) -> std::io::Result<()> {
        let contract_dir = dir.join(&self.base_name);
        fs::create_dir_all(&contract_dir)?;

        // Save ABI
        fs::write(
            contract_dir.join(format!("{}.abi.json", self.base_name)),
            &self.abi_content,
        )?;

        // Save interface
        fs::write(
            contract_dir.join(format!("I{}.sol", self.base_name)),
            &self.interface_content,
        )?;

        Ok(())
    }
}

impl ToTokens for Router {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let implementation = &self.item;
        let routes = self.routes();

        // Get struct name
        let struct_name = if let syn::Type::Path(type_path) = &*self.item.self_ty {
            type_path.path.segments.last().unwrap().ident.to_string()
        } else {
            panic!("Unexpected implementation type")
        };

        // Generate ABI
        let abi_array: Vec<Value> = routes
            .iter()
            .map(|route| serde_json::to_value(route.abi()).unwrap())
            .collect();

        // Convert to JSON string
        let abi_string = serde_json::to_string_pretty(&abi_array).unwrap();

        // Generate Solidity interface
        let interface_string = generate_solidity_interface(&struct_name, &routes);

        let artifacts = ContractArtifacts {
            base_name: struct_name.to_case(Case::Pascal),
            abi_content: serde_json::to_string_pretty(&abi_array).unwrap(),
            interface_content: generate_solidity_interface(&struct_name, &routes),
        };

        // Save artifacts
        if let Ok(out_dir) = env::var("OUT_DIR") {
            // Always save to OUT_DIR/artifacts
            let out_artifacts_dir = Path::new(&out_dir).join("artifacts");
            artifacts
                .save_to_dir(&out_artifacts_dir)
                .unwrap_or_else(|e| {
                    panic!("Failed to write artifacts to OUT_DIR: {}", e);
                });

            // If FLUENTBASE_CONTRACT_ARTIFACTS_DIR is set, also save there
            if let Ok(artifacts_dir) = env::var("FLUENTBASE_CONTRACT_ARTIFACTS_DIR") {
                artifacts
                    .save_to_dir(Path::new(&artifacts_dir))
                    .unwrap_or_else(|e| {
                        panic!(
                            "Failed to write artifacts to FLUENTBASE_CONTRACT_ARTIFACTS_DIR: {}",
                            e
                        );
                    });
            }
        }

        // Generate constant names
        let abi_const_name = format_ident!("{}_ABI", struct_name.to_case(Case::ScreamingSnake));
        let interface_const_name =
            format_ident!("{}_INTERFACE", struct_name.to_case(Case::ScreamingSnake));

        let method_codecs = routes
            .iter()
            .map(|route| route.generate_codec_impl(&self.mode))
            .collect::<Vec<_>>();

        let router = self.generate();

        output.extend(quote! {
            #implementation
            #(#method_codecs)*
            #router
            #[doc = "Complete contract ABI"]
            pub const #abi_const_name: &str = #abi_string;
            #[doc = "Solidity interface"]
            pub const #interface_const_name: &str = #interface_string;
        });
    }
}

fn generate_solidity_interface(contract_name: &str, routes: &[&Route]) -> String {
    let mut interface = format!(
        "// SPDX-License-Identifier: MIT\n\
         // This file is auto-generated\n\
         // DO NOT EDIT THIS FILE MANUALLY!\n\
         // Source contract: {}\n\
         // Generated at: {}\n\n\
         pragma solidity ^0.8.0;\n\n\
         interface I{} {{\n",
        contract_name,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        contract_name.to_case(Case::Pascal)
    );

    // Add all function definitions
    for route in routes {
        let function_def = route.abi().to_solidity_interface();
        interface.push_str(&format!("{}\n", function_def));
    }

    interface.push_str("}\n");
    interface
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_quote, ImplItem};

    #[test]
    fn test_parse_routes_trait_impl() {
        let items: Vec<ImplItem> = vec![parse_quote! {
            #[function_id("0x904fd06e")]
            fn greeting(&self, to: String, data: String) -> (bool, bool) {
                (data == "Hello, World!!".to_string(), true)
            }
        }];

        let routes = parse_routes(&items, true);
        assert!(routes.is_ok());
        let routes = routes.unwrap();
        assert_eq!(routes.len(), 1);
    }

    #[test]
    fn test_parse_routes_regular_impl() {
        let items: Vec<ImplItem> = vec![
            parse_quote! {
                pub fn public_method(&self) -> bool { true }
            },
            parse_quote! {
                fn private_method(&self) { }
            },
        ];

        let routes = parse_routes(&items, false);
        assert!(routes.is_ok());
        let routes = routes.unwrap();
        assert_eq!(routes.len(), 1); // Only public method should be included
    }

    #[test]
    fn test_regular_impl_with_fallback() {
        let items: Vec<ImplItem> = vec![
            parse_quote! {
                pub fn public_method(&self) -> bool { true }
            },
            parse_quote! {
                pub fn fallback(&mut self) { }
            },
        ];

        let routes = parse_routes(&items, false);
        assert!(routes.is_ok());
        let routes = routes.unwrap();
        assert_eq!(routes.len(), 2);
        assert!(check_fallback(&routes));
    }

    #[test]
    fn test_skip_deploy_method() {
        let items: Vec<ImplItem> = vec![
            parse_quote! {
                pub fn public_method(&self) -> bool { true }
            },
            parse_quote! {
                pub fn deploy(&mut self) { }
            },
        ];

        let routes = parse_routes(&items, false);
        assert!(routes.is_ok());
        let routes = routes.unwrap();
        assert_eq!(routes.len(), 1); // deploy method should be skipped
    }
}

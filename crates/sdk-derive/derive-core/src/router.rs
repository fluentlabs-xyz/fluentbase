use crate::{mode::RouterMode, route::Route};
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::emit_error;
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
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
        let ast: ItemImpl = input.parse()?;
        let mut routes = Vec::new();

        for item in &ast.items {
            if let ImplItem::Fn(method) = item {
                match syn::parse2::<Route>(quote! { #method }) {
                    Ok(route) => routes.push(route),
                    Err(error) => {
                        emit_error!(error.span(), "{}", error.to_string());
                    }
                }
            }
        }

        Ok(Router {
            mode: RouterMode::Solidity,
            implementation: ast,
            method_routes: routes,
            has_fallback_handler: false,
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

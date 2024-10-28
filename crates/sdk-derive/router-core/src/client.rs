use crate::{
    args::RouterArgs,
    codec::CodecGenerator,
    function_id::FunctionIDAttribute,
    mode::RouterMode,
    route::{MethodParameter, Route},
};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    Attribute,
    FnArg,
    Ident,
    Result,
    ReturnType,
    TraitItem,
    TraitItemFn,
    Type,
};

pub struct ClientMethod {
    route: Route,
    mode: RouterMode,
}

impl ClientMethod {
    fn new(method: &TraitItemFn, mode: RouterMode) -> Result<Self> {
        // Convert TraitItemFn to ImplItemFn for Route compatibility
        let impl_method: syn::ImplItemFn = parse_quote! {
            #[doc(hidden)]
            pub #method
        };

        let mut route = Route::new(&impl_method)?;

        // Process function_id attributes if present
        for attr in &method.attrs {
            if attr.path().is_ident("function_id") {
                let function_id_attr = attr.parse_args::<FunctionIDAttribute>()?;
                route.function_id_attr = Some(function_id_attr);
                break;
            }
        }

        Ok(ClientMethod { route, mode })
    }

    fn generate_method_impl(&self) -> TokenStream2 {
        let sdk_crate_name = if std::env::var("CARGO_PKG_NAME").unwrap() == "fluentbase-sdk" {
            quote! { crate }
        } else {
            quote! { fluentbase_sdk }
        };

        let sig = self.route.sig();
        let fn_name = &sig.ident;
        let selector = &self.route.function_id;

        // Generate codec implementation
        let codec = self.route.generate_codec_impl(&self.mode);

        let param_names = self.route.args.iter().map(|arg| &arg.ident);
        let call_struct = format_ident!("{}Call", self.route.fn_name);
        let return_struct = format_ident!("{}Return", self.route.fn_name);

        // Generate the actual method implementation
        let method_impl = quote! {
            #sig {
                let mut input = ::alloc::vec![0u8; 4];
                input.copy_from_slice(&[#(#selector,)*]);

                let call = #call_struct::new((#(#param_names,)*));
                input.extend(call.encode());

                let (result, exit_code) = self.sdk.contracts.call_system_contract(
                    &self.address,
                    &input,
                    self.fuel
                );

                if exit_code != 0 {
                    panic!("call failed with exit code: {}", exit_code);
                }

                if !result.is_empty() {
                    #return_struct::decode(&result)
                        .expect("failed to decode result")
                        .0
                } else {
                    Default::default()
                }
            }
        };

        quote! {
            #codec
            #method_impl
        }
    }
}

pub struct ClientGenerator {
    pub args: RouterArgs,
    trait_ast: syn::ItemTrait,
}

impl Parse for ClientGenerator {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let args = if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("client")) {
            attr.parse_args::<RouterArgs>()?
        } else {
            RouterArgs::new(RouterMode::default())
        };

        let trait_ast = input.parse()?;
        Ok(ClientGenerator { args, trait_ast })
    }
}

impl ClientGenerator {
    fn generate_client(&self) -> TokenStream2 {
        let sdk_crate_name = if std::env::var("CARGO_PKG_NAME").unwrap() == "fluentbase-sdk" {
            quote! { crate }
        } else {
            quote! { fluentbase_sdk }
        };

        let trait_name = &self.trait_ast.ident;
        let client_name = self.get_client_name();

        let methods = self
            .trait_ast
            .items
            .iter()
            .filter_map(|item| {
                if let TraitItem::Fn(method) = item {
                    Some(ClientMethod::new(method, self.args.mode()))
                } else {
                    None
                }
            })
            .collect::<Result<Vec<_>>>()
            .unwrap_or_default();

        let method_impls = methods.iter().map(|method| method.generate_method_impl());

        quote! {
            #[derive(Debug)]
            pub struct #client_name {
                pub address: #sdk_crate_name::Address,
                pub sdk: #sdk_crate_name::SDK,
                pub fuel: u32,
            }

            impl #client_name {
                pub fn new(
                    address: #sdk_crate_name::Address,
                    sdk: #sdk_crate_name::SDK
                ) -> impl #trait_name {
                    Self {
                        address,
                        sdk,
                        fuel: u32::MAX,
                    }
                }

                pub fn with_fuel(
                    address: #sdk_crate_name::Address,
                    sdk: #sdk_crate_name::SDK,
                    fuel: u32
                ) -> impl #trait_name {
                    Self {
                        address,
                        sdk,
                        fuel,
                    }
                }
            }

            impl #trait_name for #client_name {
                #(#method_impls)*
            }
        }
    }

    fn get_client_name(&self) -> Ident {
        let mut ident_name = self.trait_ast.ident.to_string();
        if ident_name.ends_with("API") {
            ident_name = ident_name.trim_end_matches("API").to_string();
        }
        Ident::new(&format!("{}Client", ident_name), Span::call_site())
    }
}

impl ToTokens for ClientGenerator {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let trait_ast = &self.trait_ast;
        let client_impl = self.generate_client();

        tokens.extend(quote! {
            #trait_ast
            #client_impl
        });
    }
}

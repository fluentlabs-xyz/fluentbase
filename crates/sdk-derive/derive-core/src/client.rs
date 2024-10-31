use crate::{args::RouterArgs, function_id::FunctionIDAttribute, mode::RouterMode, route::Route};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    Attribute,
    Result,
    TraitItem,
    TraitItemFn,
};

pub struct ClientMethod {
    route: Route,
    mode: RouterMode,
}

impl ClientMethod {
    fn new(method: &TraitItemFn, mode: RouterMode) -> Result<Self> {
        let attrs = &method.attrs;

        let mut route = Route::from_trait_fn(method)?;

        for attr in attrs {
            if attr.path().is_ident("function_id") {
                route.function_id_attr = Some(attr.parse_args::<FunctionIDAttribute>()?);
            }
        }

        Ok(ClientMethod { route, mode })
    }

    fn generate_codecs(&self) -> TokenStream2 {
        self.route.generate_codec_impl(&self.mode)
    }

    fn generate_helpers(&self) -> TokenStream2 {
        let fn_name = &self.route.sig().ident;

        let param_names = self
            .route
            .args
            .iter()
            .map(|arg| &arg.ident)
            .collect::<Vec<_>>();
        let param_types = self
            .route
            .args
            .iter()
            .map(|arg| &arg.ty)
            .collect::<Vec<_>>();

        let return_types = &param_types;

        let fn_name_str = fn_name.to_string();
        let encode_name = format_ident!("encode_{}", fn_name_str);
        let decode_name = format_ident!("decode_{}", fn_name_str);

        let pascal_name = self.route.fn_name.to_case(Case::Pascal);
        let call_struct = format_ident!("{}Call", pascal_name);

        // TODO: d1r1 - add return struct
        let return_struct = format_ident!("{}Return", pascal_name);

        quote! {
            pub fn #encode_name(
                &self,
                #(#param_names: #param_types,)*
            ) -> fluentbase_sdk::Bytes {
                 #call_struct::new((#(#param_names,)*)).encode().into()
            }

            pub fn #decode_name(
                &self,
                output: fluentbase_sdk::Bytes
            ) -> (#(#return_types,)*) {
                #call_struct::decode(&output)
                    .expect("failed to decode result")
                    .0
            }
        }
    }

    fn generate_implementation(&self) -> TokenStream2 {
        let fn_name = &self.route.sig().ident;
        let param_names = self
            .route
            .args
            .iter()
            .map(|arg| &arg.ident)
            .collect::<Vec<_>>();

        let param_types = self
            .route
            .args
            .iter()
            .map(|arg| &arg.ty)
            .collect::<Vec<_>>();
        let return_types = &param_types;

        let fn_name_str = fn_name.to_string();
        let encode_name = format_ident!("encode_{}", fn_name_str);
        let decode_name = format_ident!("decode_{}", fn_name_str);

        quote! {
            pub fn #fn_name(
                &mut self,
                contract_address: fluentbase_sdk::Address,
                value: fluentbase_sdk::U256,
                gas_limit: u64,
                #(#param_names: #param_types,)*
            ) -> (#(#return_types,)*) {
                let input = self.#encode_name(#(#param_names,)*);

                let tx_context = self.sdk.tx_context();
                if tx_context.value < value {
                    ::core::panic!("insufficient funds");
                }
                if tx_context.gas_limit < gas_limit {
                    ::core::panic!("insufficient gas");
                }

                let (output, exit_code) = self.sdk.call(
                    contract_address,
                    value,
                    &input,
                    gas_limit
                );

                if exit_code != 0 {
                    ::core::panic!("call failed");
                }

                self.#decode_name(output)
            }
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
        let trait_name = &self.trait_ast.ident;
        let client_name = format_ident!("{}Client", trait_name);

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

        let codecs = methods.iter().map(|method| method.generate_codecs());
        let helpers = methods.iter().map(|method| method.generate_helpers());
        let implementations = methods
            .iter()
            .map(|method| method.generate_implementation());

        quote! {
            // Client structure
            pub struct #client_name<SDK> {
                pub sdk: SDK,
            }

            // Codec implementations
            #(#codecs)*

            // Helper functions
            impl<SDK: fluentbase_sdk::SharedAPI> #client_name<SDK> {
                pub fn new(sdk: SDK) -> Self {
                    Self { sdk }
                }

                #(#helpers)*
            }

            // Main interface implementation
            impl<SDK: fluentbase_sdk::SharedAPI> #client_name<SDK> {
                #(#implementations)*
            }
        }
    }
}

impl ToTokens for ClientGenerator {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let client_impl = self.generate_client();
        tokens.extend(client_impl);
    }
}

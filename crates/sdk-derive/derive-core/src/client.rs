use crate::{
    abi::structs::StructResolver,
    attr::{mode::Mode, StateMutabilityExt},
    codec::CodecGenerator,
    method::{MethodCollector, MethodLike, ParsedMethod},
};
use convert_case::{Case, Casing};
use darling::{ast::NestedMeta, FromMeta};
use proc_macro2::{Span, TokenStream as TokenStream2};
use proc_macro_error::{abort, abort_call_site, emit_error};
use quote::{format_ident, quote, ToTokens};
use syn::{spanned::Spanned, visit, Error, Ident, ItemTrait, Result, TraitItemFn};

/// Attributes for the client configuration.
#[derive(Debug, FromMeta, Default, Clone)]
pub struct ClientAttributes {
    /// The client mode (Solidity or Fluent)
    pub mode: Mode,
}

/// A client that provides function calls to contracts.
/// Supports both Solidity-compatible and Fluent API modes.
#[derive(Debug)]
pub struct Client<T: MethodLike> {
    /// Client configuration attributes
    attributes: ClientAttributes,
    /// The original trait definition
    trait_def: ItemTrait,
    /// Collection of available method routes
    methods: Vec<ParsedMethod<T>>,
}

/// Parses and validates a client from token streams.
pub fn process_client(attr: TokenStream2, input: TokenStream2) -> Result<Client<TraitItemFn>> {
    process_client_with_structs(attr, input, &StructResolver::crate_sources())
}

/// Parses and validates a client, resolving struct parameters through the given resolver.
pub fn process_client_with_structs(
    attr: TokenStream2,
    input: TokenStream2,
    resolver: &StructResolver,
) -> Result<Client<TraitItemFn>> {
    let attributes = parse_attributes(attr)?;

    let trait_def = syn::parse2::<ItemTrait>(input)?;

    let client = Client::new(attributes, trait_def, resolver)?;

    Ok(client)
}

/// Parses client attributes from a TokenStream.
fn parse_attributes(attr: TokenStream2) -> Result<ClientAttributes> {
    let meta = NestedMeta::parse_meta_list(attr)?;
    ClientAttributes::from_list(&meta).map_err(|e| Error::new(Span::call_site(), e.to_string()))
}

impl Client<TraitItemFn> {
    pub fn new(
        attributes: ClientAttributes,
        trait_def: ItemTrait,
        resolver: &StructResolver,
    ) -> Result<Self> {
        let mut collector = MethodCollector::<TraitItemFn>::new(trait_def.span(), resolver);
        visit::visit_item_trait(&mut collector, &trait_def);

        if collector.methods.is_empty() {
            abort!(
                collector.span,
                "Client trait has no methods. Make sure your trait contains at least one method that is not named 'deploy' or 'fallback'.";
                help = "A valid trait should contain at least one method declaration";
                help = "Example: fn method_name(&self, param: Type) -> ReturnType;"
            );
        }

        if collector.has_errors() {
            for err in &collector.errors {
                emit_error!(err.span(), "{}", err.to_string());
            }

            abort_call_site!("Failed to process client trait due to method parsing errors");
        }

        // Finally check for selector collisions
        if let Err(collision_error) = collector.validate_selectors() {
            abort!(
                collision_error.span(),
                "{}",
                collision_error.to_string();
                help = "Function selectors must be unique across all methods in the trait";
                help = "You can use custom selectors with #[function_id(\"custom_signature\")]";
                help = "Or rename your methods to have different signatures"
            );
        }

        Ok(Self {
            attributes,
            trait_def,
            methods: collector.methods,
        })
    }
}

impl<T: MethodLike> Client<T> {
    /// Returns the client attributes.
    pub fn attributes(&self) -> &ClientAttributes {
        &self.attributes
    }

    /// Returns the trait definition.
    pub fn trait_def(&self) -> &ItemTrait {
        &self.trait_def
    }

    /// Returns the trait name
    pub fn trait_name(&self) -> &Ident {
        &self.trait_def.ident
    }

    /// Returns the client name (typically TraitName + "Client")
    pub fn client_name(&self) -> Ident {
        format_ident!("{}Client", self.trait_name())
    }

    /// Returns the list of parsed methods.
    pub fn methods(&self) -> &[ParsedMethod<T>] {
        &self.methods
    }

    /// Generates the complete client implementation with all artifacts.
    pub fn generate(&self) -> Result<TokenStream2> {
        let client_name = self.client_name();

        let method_codecs = self.generate_codec_implementations()?;

        let client_methods = self.generate_client_methods()?;

        let output = quote! {
            #[derive(Debug)]
            pub struct #client_name<SDK> {
                pub sdk: SDK,
            }

            #(#method_codecs)*

            impl<SDK: fluentbase_sdk::SharedAPI> #client_name<SDK> {
                pub fn new(sdk: SDK) -> Self {
                    Self { sdk }
                }

                #client_methods
            }
        };

        Ok(output)
    }

    /// Generates codec implementations for all available methods.
    fn generate_codec_implementations(&self) -> Result<Vec<TokenStream2>> {
        self.methods()
            .iter()
            .map(|method| {
                CodecGenerator::new(method, &self.attributes().mode)
                    .generate()
                    .map_err(|e| Error::new(method.parsed_signature().span(), e.to_string()))
            })
            .collect()
    }

    /// Generates client methods for all available methods.
    fn generate_client_methods(&self) -> Result<TokenStream2> {
        let methods = self
            .methods()
            .iter()
            .map(|method| self.generate_client_method(method))
            .collect::<Result<Vec<_>>>()?;

        Ok(quote! {
            #(#methods)*
        })
    }

    /// Generates a single client method implementation.
    fn generate_client_method(&self, method: &ParsedMethod<T>) -> Result<TokenStream2> {
        let sig = method.parsed_signature();
        let fn_name = &sig.ident;
        let fn_args = format_ident!("{}Call", fn_name.to_string().to_case(Case::Pascal));
        let fn_return = format_ident!("{}Return", fn_name.to_string().to_case(Case::Pascal));

        // Generate parameters from signature
        let params = sig.inputs.iter().filter_map(|param| {
            if let syn::FnArg::Typed(pat_type) = param {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    let name = &pat_ident.ident;
                    let ty = &pat_type.ty;
                    Some(quote! { #name: #ty })
                } else {
                    None
                }
            } else {
                None
            }
        });

        // Get parameter names only
        let param_names = sig.inputs.iter().filter_map(|param| {
            if let syn::FnArg::Typed(pat_type) = param {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    Some(&pat_ident.ident)
                } else {
                    None
                }
            } else {
                None
            }
        });

        // Get return type from signature
        let return_type = match &sig.output {
            syn::ReturnType::Default => quote! { () },
            syn::ReturnType::Type(_, ty) => quote! { #ty },
        };

        // Generate decode output
        let decode_output = match &sig.output {
            syn::ReturnType::Default => {
                quote! {
                    ()
                }
            }
            syn::ReturnType::Type(_, ty) => {
                if let syn::Type::Tuple(_) = &**ty {
                    quote! {
                        #fn_return::decode(&result.data)
                            .expect("failed to decode result")
                            .0
                    }
                } else {
                    // Single return value
                    quote! {
                        #fn_return::decode(&result.data)
                            .expect("failed to decode result")
                            .0.0
                    }
                }
            }
        };

        // The host operation and the value policy follow the Solidity mutability:
        // `view`/`pure` must not be able to mutate state or move funds, so they
        // are issued as static calls without a value parameter, and `nonpayable`
        // keeps the mutable call but has no way to attach a value.
        let mutability = method.state_mutability();

        let value_param = if mutability.allows_value() {
            quote! { value: fluentbase_sdk::U256, }
        } else {
            quote! {}
        };

        let value_check = if mutability.allows_value() {
            quote! {
                if context.tx_value() < value {
                    ::core::panic!("Insufficient funds for transaction");
                }
            }
        } else {
            quote! {}
        };

        let host_call = if mutability.is_static() {
            quote! {
                self.sdk.static_call(
                    contract_address,
                    &input,
                    Some(gas_limit),
                )
            }
        } else {
            let value = if mutability.allows_value() {
                quote! { value }
            } else {
                quote! { fluentbase_sdk::U256::ZERO }
            };

            quote! {
                self.sdk.call(
                    contract_address,
                    #value,
                    &input,
                    Some(gas_limit),
                )
            }
        };

        Ok(quote! {
            pub fn #fn_name(
                &mut self,
                contract_address: fluentbase_sdk::Address,
                #value_param
                gas_limit: u64,
                #(#params,)*
            ) -> #return_type {
                use fluentbase_sdk::ContextReader;

                let input = fluentbase_sdk::Bytes::from(#fn_args::new((#(#param_names,)*)).encode());

                {
                    let context = self.sdk.context();
                    #value_check
                    if context.tx_gas_limit() < gas_limit {
                        ::core::panic!("Insufficient gas limit for transaction");
                    }
                }

                let result = #host_call;

                if !fluentbase_sdk::SyscallResult::is_ok(result.status) {
                    ::core::panic!("Contract call failed");
                }

                #decode_output
            }
        })
    }
}

impl ToTokens for Client<TraitItemFn> {
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
    fn test_generate_client() {
        let trait_def: ItemTrait = parse_quote! {
            pub trait TestContract {
                fn first_method(&self, value: u32) -> u32;
                fn second_method(&self, a: String, b: bool) -> (String, bool);
            }
        };

        let attributes = ClientAttributes::default();
        let client = Client::new(attributes, trait_def, &StructResolver::default()).unwrap();

        let generated = client.generate().unwrap();

        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("generate_client", formatted.to_string());
    }

    /// Generates a client for a single method and strips whitespace, so
    /// assertions can pin the exact host call without depending on formatting
    fn generated_method(method: TraitItemFn) -> String {
        let trait_def: ItemTrait = parse_quote! {
            pub trait CallPolicy {
                #method
            }
        };

        let client = Client::new(
            ClientAttributes::default(),
            trait_def,
            &StructResolver::default(),
        )
        .unwrap();
        let generated = client.generate().unwrap();

        generated
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn test_pure_and_view_methods_issue_static_calls_without_value() {
        for method in [
            parse_quote! {
                #[state_mutability("pure")]
                fn read_only(&self, a: u32) -> u32;
            },
            parse_quote! {
                #[state_mutability("view")]
                fn read_only(&self, a: u32) -> u32;
            },
            // Hand-written traits carry no attribute: `&self` is read-only
            parse_quote! {
                fn read_only(&self, a: u32) -> u32;
            },
        ] {
            let generated = generated_method(method);

            assert!(generated
                .contains("self.sdk.static_call(contract_address,&input,Some(gas_limit),)"));
            assert!(!generated.contains("self.sdk.call("));
            // No value can be attached, and none has to be checked
            assert!(!generated.contains("value:fluentbase_sdk::U256"));
            assert!(!generated.contains("tx_value()"));
        }
    }

    #[test]
    fn test_nonpayable_methods_call_with_zero_value() {
        let generated = generated_method(parse_quote! {
            #[state_mutability("nonpayable")]
            fn mutate(&mut self, a: u32) -> u32;
        });

        assert!(generated.contains(
            "self.sdk.call(contract_address,fluentbase_sdk::U256::ZERO,&input,Some(gas_limit),)"
        ));
        assert!(!generated.contains("static_call"));
        assert!(!generated.contains("value:fluentbase_sdk::U256"));
        assert!(!generated.contains("tx_value()"));
    }

    #[test]
    fn test_payable_methods_forward_the_requested_value() {
        for method in [
            parse_quote! {
                #[state_mutability("payable")]
                fn mutate(&mut self, a: u32) -> u32;
            },
            // Hand-written traits carry no attribute: `&mut self` keeps the
            // historical behavior of forwarding a caller-supplied value
            parse_quote! {
                fn mutate(&mut self, a: u32) -> u32;
            },
        ] {
            let generated = generated_method(method);

            assert!(generated.contains("value:fluentbase_sdk::U256,gas_limit:u64"));
            assert!(
                generated.contains("self.sdk.call(contract_address,value,&input,Some(gas_limit),)")
            );
            assert!(generated.contains("context.tx_value()<value"));
            assert!(!generated.contains("static_call"));
        }
    }
}

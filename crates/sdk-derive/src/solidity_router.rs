use crate::utils::{
    calculate_keccak256_bytes,
    get_all_methods,
    get_public_methods,
    get_raw_signature,
    get_signatures,
    sol_call_fn_name,
};
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input,
    FnArg,
    Ident,
    ImplItemFn,
    ItemImpl,
    ItemTrait,
    ReturnType,
    TraitItem,
    TraitItemFn,
};

pub fn derive_solidity_router(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast: ItemImpl = parse_macro_input!(item as ItemImpl);
    let struct_name = &ast.self_ty;
    let generics = &ast.generics;

    let all_methods = get_all_methods(&ast);
    let public_methods = get_public_methods(&ast);

    // Dispatch all methods (public and private) if implementing a trait
    let methods_to_dispatch = if ast.trait_.is_some() {
        all_methods
            .clone()
            .into_iter()
            .filter(|func| func.sig.ident != "deploy")
            .collect()
    } else {
        public_methods.clone()
    };

    // Generate Solidity function signatures or use provided ones from #[signature]
    let signatures = get_signatures(&methods_to_dispatch);

    // Derive route method that dispatches Solidity function calls
    let router_impl = derive_route_method(&methods_to_dispatch);

    let expanded = quote! {
        use alloy_sol_types::{sol, SolCall, SolValue};
        #signatures

        #ast

        impl #generics #struct_name {
            #router_impl
        }
    };

    TokenStream::from(expanded)
}

fn derive_route_method(methods: &Vec<&ImplItemFn>) -> proc_macro2::TokenStream {
    let selectors: Vec<proc_macro2::TokenStream> = methods
        .iter()
        .filter_map(|method| {
            let selector = derive_route_selector_arm(method);
            Some(selector)
        })
        .collect();

    let match_arms = if selectors.is_empty() {
        quote! {
            _ => panic!("No methods to route"),
        }
    } else {
        quote! {
            #(#selectors),*,
            _ => panic!("unknown method"),
        }
    };

    quote! {
        pub fn main(&self) {
            let input_size = self.sdk.input_size();
            if input_size < 4 {
                panic!("input too short, cannot extract selector");
            }
            let mut selector: [u8; 4] = [0; 4];
            self.sdk.read(&mut selector, 0);
            let input = fluentbase_sdk::alloc_slice(input_size as usize);
            self.sdk.read(input, 0);
            match selector {
                #match_arms
            }
        }
    }
}

fn derive_route_selector_arm(func: &ImplItemFn) -> proc_macro2::TokenStream {
    let method_name = &func.sig.ident;
    let (_impl_generics, type_generics, _where_clause) = func.sig.generics.split_for_impl();
    let method_name_call = sol_call_fn_name(method_name);
    let selector_name = quote! { #method_name_call::SELECTOR };
    let abi_decode = quote! { #method_name_call::abi_decode };

    let generics = if func.sig.generics.params.is_empty() {
        quote!()
    } else {
        quote!(::#type_generics)
    };

    let args: Vec<_> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    Some(&pat_ident.ident)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    let args_expr = derive_route_selector_args(&args, &abi_decode);

    quote! {
        #selector_name => {
            #args_expr
            let output = self.#method_name #generics(#(#args),*).abi_encode();
            self.sdk.write(&output);
        }
    }
}

fn derive_route_selector_args(
    args: &[&Ident],
    abi_decode_fn: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if args.len() == 1 {
        let arg = args[0];
        quote! {
            let #arg = match #abi_decode_fn(&input, true) {
                Ok(decoded) => decoded.#arg,
                Err(_) => panic!("failed to decode input"),
            };
        }
    } else if args.len() > 0 {
        let fields: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| quote! { decoded.#arg }).collect();
        quote! {
            let (#(#args),*) = match #abi_decode_fn(&input, true) {
                Ok(decoded) => (#(#fields),*),
                Err(_) => panic!("failed to decode input"),
            };
        }
    } else {
        quote! {}
    }
}

pub fn derive_solidity_client(_attr: TokenStream, ast: ItemTrait) -> TokenStream {
    let items = ast
        .items
        .iter()
        .filter_map(|item| {
            if let TraitItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect::<Vec<&TraitItemFn>>();

    let sdk_crate_name = if std::env::var("CARGO_PKG_NAME").unwrap() == "fluentbase-sdk" {
        quote! { crate }
    } else {
        quote! { fluentbase_sdk }
    };

    let mut methods = Vec::new();
    for item in items {
        let sig = &item.sig;
        let mut inputs = Vec::new();
        for arg in sig.inputs.iter() {
            let arg = match arg {
                FnArg::Receiver(_) => continue,
                FnArg::Typed(arg) => &arg.pat,
            };
            inputs.push(quote! { #arg });
        }
        let outputs = match &sig.output {
            ReturnType::Default => {
                quote! {}
            }
            ReturnType::Type(_, ty) => {
                quote! { #ty::abi_decode(&result, false).expect("failed to decode result") }
            }
        };
        let sol_sig = get_raw_signature(item);
        let sol_sig = calculate_keccak256_bytes(sol_sig.to_string().as_str());
        let method = quote! {
            #sig {
                use alloy_sol_types::{SolValue};
                let mut input = alloc::vec![0u8; 4];
                input.copy_from_slice(&[#( #sol_sig, )*]);
                let input_args = (#( #inputs, )*).abi_encode();
                input.extend(input_args);
                let (result, exit_code) = #sdk_crate_name::contracts::call_system_contract(&self.address, &input, self.fuel);
                if exit_code != 0 {
                    panic!("call failed with exit code: {}", exit_code)
                }
                #outputs
            }
        };
        methods.push(method);
    }

    let mut ident_name = ast.ident.to_string();
    if ident_name.ends_with("API") {
        ident_name = ident_name.trim_end_matches("API").to_string();
    }
    let client_name = Ident::new((ident_name + "Client").as_str(), ast.ident.span());
    let trait_name = &ast.ident;

    let expanded = quote! {
        #ast
        pub struct #client_name {
            pub address: #sdk_crate_name::Address,
            pub fuel: u32,
        }
        impl #client_name {
            pub fn new(address: #sdk_crate_name::Address) -> impl #trait_name {
                Self { address, fuel: u32::MAX }
            }
        }
        impl #trait_name for #client_name {
            #( #methods )*
        }
    };

    TokenStream::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::rust_name_to_sol;
    use syn::{parse_quote, ImplItem};

    #[test]
    fn test_get_signatures_full_signature() {
        let item_impl: ItemImpl = parse_quote! {
            impl ExampleStruct {
                #[signature("function greeting(string message) external returns (string)")]
                fn greeting(&self, message: String) -> String {
                    message
                }
            }
        };

        let methods = item_impl
            .items
            .iter()
            .filter_map(|item| {
                if let ImplItem::Fn(func) = item {
                    Some(func)
                } else {
                    None
                }
            })
            .collect::<Vec<&ImplItemFn>>();

        let signatures = get_signatures(&methods);

        let expected = quote! {
            sol! {
                function greeting(string message) external returns (string);
            }
        };

        assert_eq!(signatures.to_string(), expected.to_string());
    }

    #[test]
    fn test_get_signatures_short_signature() {
        let item_impl: ItemImpl = parse_quote! {
            impl ExampleStruct {
                #[signature("customGreeting(string)")]
                fn custom_greeting(&self, message: String) -> String {
                    message
                }
            }
        };

        let methods = item_impl
            .items
            .iter()
            .filter_map(|item| {
                if let ImplItem::Fn(func) = item {
                    Some(func)
                } else {
                    None
                }
            })
            .collect::<Vec<&ImplItemFn>>();

        let signatures = get_signatures(&methods);

        let expected = quote! {
            sol! {
                function customGreeting(string message) external returns (string);
            }
        };

        assert_eq!(signatures.to_string(), expected.to_string());
    }

    #[test]
    fn test_derive_route_selector_arm() {
        let func: ImplItemFn = parse_quote! {
            pub fn greet(&self, msg: String) -> String {
                msg
            }
        };

        let expected = quote! {
            greetCall::SELECTOR => {
                let msg = match greetCall::abi_decode(&input, true) {
                    Ok(decoded) => decoded.msg,
                    Err(_) => panic!("failed to decode input"),
                };
                let output = self.greet(msg).abi_encode();
                self.sdk.write(&output);
            }
        };

        let actual = derive_route_selector_arm(&func);
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_get_signatures() {
        let item_impl: ItemImpl = parse_quote! {
            impl ExampleStruct {
                #[signature("function greeting() external view returns ()")]
                pub fn greeting(&self, msg: String) -> String {
                    msg
                }

                pub fn regular_fn(&self, msg: String) -> String {
                    msg
                }
                pub fn greeting_msg(&self, msg: String) -> String {
                    msg
                }
            }
        };

        let methods = get_public_methods(&item_impl);
        let signatures = get_signatures(&methods);

        let expected = quote! {
            sol! {
                function greeting() external view returns ();
                function regularFn(string msg) external returns (string);
                function greetingMsg(string msg) external returns (string);
            }
        };

        assert_eq!(signatures.to_string(), expected.to_string());
    }

    #[test]
    fn test_rust_name_to_sol() {
        let ident = Ident::new("test_function", proc_macro2::Span::call_site());
        let sol_ident = rust_name_to_sol(&ident);
        assert_eq!(sol_ident.to_string(), "testFunction");
    }

    #[test]
    fn test_get_method_call() {
        let method_name = Ident::new("test_function", proc_macro2::Span::call_site());
        let method_call_ident = sol_call_fn_name(&method_name);
        assert_eq!(method_call_ident.to_string(), "testFunctionCall");
    }
}

use crate::utils::{
    calculate_keccak256_bytes,
    get_all_methods,
    get_public_methods,
    get_raw_signature,
    parse_function_inputs,
    rust_name_to_sol,
    rust_type_to_sol,
};
use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Literal, Punct, Spacing, Span, TokenStream as TokenStream2};
use proc_macro_error::emit_error;
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{
    parse::{Parse, ParseStream, Parser},
    parse_macro_input,
    parse_str,
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute,
    Error,
    FnArg,
    Ident,
    ImplItem,
    ImplItemFn,
    ItemImpl,
    ItemTrait,
    Lit,
    LitStr,
    Meta,
    Result,
    ReturnType,
    Token,
    TraitItem,
    TraitItemFn,
    Type,
};
use syn_solidity::{parse2, File, FunctionBody, Item as SolidityItem};

pub fn derive_solidity_router(input: TokenStream) -> Result<TokenStream> {
    let input = TokenStream2::from(input);
    let ast: ItemImpl = syn::parse2(input)?;

    // Parse all routes from the provided implementation
    let routes = parse_all_methods(&ast)?;

    // If implementing router for trait, use all methods; otherwise, filter out non-public methods
    // and 'deploy'
    let methods_to_route: Vec<Route> = if ast.trait_.is_some() {
        routes.clone()
    } else {
        routes
            .clone()
            .into_iter()
            .filter(|r| r.is_public && r.fn_name != "deploy")
            .collect()
    };

    // Check if the fallback method exists
    let has_fallback = methods_to_route.iter().any(|m| m.fn_name == "fallback");

    println!("has fallback: {}", has_fallback);

    // Exclude fallback from the routes and create selectors for routing
    let routes_except_fallback: Vec<Route> = methods_to_route
        .iter()
        .filter(|r| r.fn_name != "fallback")
        .cloned()
        .collect();

    // Generate the main router code
    let router = generate_router(&ast, &routes_except_fallback, has_fallback);

    let sol_signatures = routes_except_fallback
        .iter()
        .map(|r| {
            let sol_code = r.signature.clone();
            let token_stream: TokenStream2 = sol_code
                .parse()
                .expect("Failed to convert string to TokenStream2");
            return token_stream;
        })
        .collect::<Vec<_>>();

    let output = quote! {
        #ast

        use alloy_sol_types::{sol, SolCall, SolValue};

        sol! {
            #(#sol_signatures);*;
        }

        #router
    };
    Ok(output.into())
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
        let sol_sig = calculate_keccak256_bytes::<4>(sol_sig.to_string().as_str());
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

    TokenStream2::from(expanded).into()
}

fn parse_all_methods(ast: &ItemImpl) -> Result<Vec<Route>> {
    let mut routes = Vec::new();
    let mut errors = Vec::new();

    for item in &ast.items {
        if let ImplItem::Fn(method) = item {
            match syn::parse2::<Route>(quote! { #method }) {
                Ok(route) => routes.push(route),
                Err(e) => errors.push(Error::new(
                    method.span(),
                    format!("Failed to parse method: {}; due: {}", &method.sig.ident, e),
                )),
            }
        }
    }

    if !errors.is_empty() {
        let combined_error = errors
            .into_iter()
            .reduce(|mut acc, e| {
                acc.combine(e);
                acc
            })
            .unwrap();
        return Err(combined_error);
    }
    Ok(routes)
}

fn generate_router(ast: &ItemImpl, routes: &[Route], has_fallback: bool) -> TokenStream2 {
    let struct_name = &ast.self_ty;
    let generics = &ast.generics;

    // Generate token streams for each route
    let routes_tokens: Vec<TokenStream2> =
        routes.iter().map(|route| route.to_token_stream()).collect();

    // Add fallback handling
    let fallback_arm = if has_fallback {
        quote! {
            _ => {
                self.fallback();
            },
        }
    } else {
        quote! {
            _ => panic!("unknown method"),
        }
    };

    // If no routes are available, generate an error
    let match_arms = if routes.is_empty() {
        quote! {
            _ => panic!("No methods to route"),
        }
    } else {
        quote! {
            #(#routes_tokens),*,
            #fallback_arm
        }
    };

    let input_size_check = if has_fallback {
        quote! {
            if input_size < 4 {
                self.fallback();
            }
        }
    } else {
        quote! {
            if input_size < 4 {
                panic!("input too short, cannot extract selector");
            }
        }
    };

    // Final routing function
    quote! {
        impl #generics #struct_name {
            pub fn main(&mut self) {
                let input_size = self.sdk.input_size();
                #input_size_check;

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
}

#[derive(Clone, Debug)]
struct Route {
    pub function_id: [u8; 4],
    pub fn_name: String,
    pub signature: String,
    pub args: Vec<Arg>,
    pub is_public: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Arg {
    pub ty: Type,
    pub ident: Ident,
}

#[derive(Debug)]
struct SignatureAttribute {
    pub signature: String,
    pub validate: bool,
}
impl Parse for Route {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let mut function_id_attr = None;
        let mut signature_attr = None;

        for attr in &attrs {
            if attr.path().is_ident("signature") {
                signature_attr = Some(attr.parse_args::<SignatureAttribute>()?);
            } else if attr.path().is_ident("function_id") {
                let lit: LitStr = attr.parse_args()?;
                function_id_attr = Some(parse_function_id(&lit.value())?);
            }
        }

        let method: ImplItemFn = input.parse()?;
        let method_sol_signature = get_full_sol_signature(&method);

        let args: Vec<Arg> = Arg::from(&method);

        let function_id = if let Some(id) = function_id_attr {
            id
        } else if let Some(sig_attr) = signature_attr {
            if sig_attr.validate {
                validate_solidity_signature(&sig_attr.signature).map_err(|e| {
                    Error::new(
                        Span::call_site(),
                        format!("Invalid Solidity function signature: {}", e),
                    )
                })?;
            }

            calculate_keccak256_bytes(&sig_attr.signature)
        } else {
            get_function_id(&method)
        };
        let is_public = match method.vis {
            syn::Visibility::Public(_) => true,
            _ => false,
        };

        eprintln!("Function ID: {:?}", function_id);
        eprintln!("Signature: {:?}", &get_minimal_sol_signature(&method));

        Ok(Route {
            function_id,
            fn_name: method.sig.ident.to_string(),
            signature: method_sol_signature,
            args,
            is_public,
        })
    }
}

impl Parse for SignatureAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let sig_str: LitStr = input.parse()?;
        let mut validate = true;

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            let validate_lit: Lit = input.parse()?;
            match validate_lit {
                Lit::Bool(lit_bool) => validate = lit_bool.value,
                _ => return Err(input.error("Expected boolean literal for validate")),
            }
        }

        Ok(SignatureAttribute {
            signature: sig_str.value(),
            validate,
        })
    }
}

// TO TOKENS

impl ToTokens for Route {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        tokens.extend(self.to_token_stream());
    }
}

impl Route {
    fn to_token_stream(&self) -> TokenStream2 {
        let selector: Vec<TokenStream2> = self
            .function_id
            .iter()
            .map(|byte| {
                let val = format!("{}u8", byte);
                let lit = syn::LitInt::new(&val, Span::call_site());

                quote! { #lit }
            })
            .collect();

        let input = self.decode_input_args();
        let method_name = Ident::new(&self.fn_name, Span::call_site());

        let fn_call_args = self.get_function_call_args();

        quote! {
            [#(#selector),*] => {
                #input
                let output = self.#method_name(#fn_call_args).abi_encode();
                self.sdk.write(&output);
            }
        }
    }

    fn decode_input_args(&self) -> TokenStream2 {
        if self.args.is_empty() {
            quote! {}
        } else {
            let fn_name = Ident::new(&self.fn_name, Span::call_site());

            let fn_call_name = Ident::new(
                &format!("{}Call", rust_name_to_sol(&fn_name)),
                Span::call_site(),
            );
            let decode_args: Vec<TokenStream2> =
                self.args.iter().map(|arg| arg.to_decode_token()).collect();

            quote! {
                // FIXME:
                // TODO: d1r1
                // abi_decode validates input function_id.
                // so we can't use our custom function_id here
                // we need to restore it to the #fn_call_name::SELECTOR
                input[0..4].copy_from_slice(&#fn_call_name::SELECTOR);
                let (#(#decode_args),*) = match #fn_call_name::abi_decode(&input, true) {
                    Ok(decoded) => (
                        #(decoded.#decode_args),*
                    ),
                    Err(_) => panic!("failed to decode input"),
                };
            }
        }
    }

    fn get_function_call_args(&self) -> TokenStream2 {
        let args = self.args.iter().map(|arg| arg.to_call_token());
        quote! { #(#args),* }
    }
}

impl Arg {
    fn from(method: &ImplItemFn) -> Vec<Self> {
        method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let FnArg::Typed(pat_type) = arg {
                    if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        Some(Arg {
                            ty: (*pat_type.ty).clone(),
                            ident: pat_ident.ident.clone(),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn to_decode_token(&self) -> TokenStream2 {
        let ident = &self.ident;
        quote! { #ident }
    }

    fn to_call_token(&self) -> TokenStream2 {
        let ident = &self.ident;
        match &self.ty {
            Type::Reference(ty_ref) if ty_ref.mutability.is_some() => {
                quote! { &mut #ident }
            }
            Type::Reference(_) => {
                quote! { &#ident }
            }
            _ => {
                quote! { #ident }
            }
        }
    }
}

// HELPERS

fn get_full_sol_signature(method: &ImplItemFn) -> String {
    let method_name = &method.sig.ident;
    let sol_method_name = rust_name_to_sol(method_name);

    let inputs = parse_function_inputs(&method.sig.inputs);
    let input_strings: Vec<String> = inputs
        .iter()
        .map(|input| input.to_token_stream().to_string())
        .collect();

    let params = input_strings.join(", ");

    let return_part = match &method.sig.output {
        ReturnType::Type(_, ty) => {
            let output = rust_type_to_sol(&ty);
            format!(" returns ({})", output)
        }
        ReturnType::Default => String::new(),
    };

    format!(
        "function {}({}) external {}",
        sol_method_name, params, return_part
    )
}

fn get_minimal_sol_signature(method: &ImplItemFn) -> String {
    let method_name = &method.sig.ident;
    let sol_method_name = rust_name_to_sol(method_name);

    let inputs = parse_function_inputs(&method.sig.inputs);
    let input_strings: Vec<String> = inputs
        .iter()
        .map(|input| input.to_token_stream().to_string())
        .collect();

    let params = input_strings.join(", ");

    format!("{}({})", sol_method_name, params)
}

fn get_function_id(method: &ImplItemFn) -> [u8; 4] {
    let method_name = &method.sig.ident;
    let sol_method_name = rust_name_to_sol(method_name);

    // Получаем только типы аргументов с помощью rust_type_to_sol
    let input_types: Vec<String> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                // Convert Rust type to Solidity type using `rust_type_to_sol`, which returns a
                // `TokenStream2`.
                let ty = &pat_type.ty;
                Some(rust_type_to_sol(ty).to_string()) // Convert the TokenStream2 to String here.
            } else {
                None // Non-typed arguments are skipped.
            }
        })
        .collect();

    let params = input_types.join(",");

    let signature = format!("{}({})", sol_method_name, params).replace(" ", "");

    calculate_keccak256_bytes::<4>(&signature)
}

fn parse_function_id(input: &str) -> Result<[u8; 4]> {
    if !input.starts_with("0x") || input.len() != 10 {
        return Err(Error::new(
            Span::call_site(),
            "function_id must be in the format '0x' followed by 8 hex characters",
        ));
    }

    // rm 0x
    let bytes = hex::decode(&input[2..])
        .map_err(|_| Error::new(Span::call_site(), "Invalid hex in function_id"))?;

    // check length
    if bytes.len() != 4 {
        return Err(Error::new(
            Span::call_site(),
            "function_id must be exactly 4 bytes long",
        ));
    }

    let mut result = [0u8; 4];
    result.copy_from_slice(&bytes);
    Ok(result)
}
// fn validate_signature(signature: &str) -> Result<()> {
//     let signature_tokens: TokenStream2 = signature.parse()?;

//     let file: File = parse2(signature_tokens).map_err(|e| {
//         Error::new(
//             Span::call_site(),
//             format!("Failed to parse signature: {}", e),
//         )
//     })?;

//     let item = file
//         .items
//         .into_iter()
//         .next()
//         .ok_or_else(|| Error::new(Span::call_site(), "No items found in parsed file"))?;

//     let function = match item {
//         SolidityItem::Function(func) => func,
//         _ => {
//             return Err(Error::new(
//                 Span::call_site(),
//                 "Parsed item is not a function",
//             ))
//         }
//     };

//     if function.name.is_none() {
//         return Err(Error::new(Span::call_site(), "Function name is empty"));
//     }

//     eprintln!("signature is valid");

//     Ok(())
// }

use regex::Regex;

fn validate_solidity_signature(signature: &str) -> core::result::Result<(), String> {
    let short_result = validate_short_signature(signature);
    let full_result = parse_full_signature(signature);

    match (short_result, full_result) {
        (Ok(_), _) | (_, Ok(_)) => Ok(()),
        (Err(short_err), Err(full_err)) => Err(format!(
            "Invalid signature. Neither short nor full format is valid.\n\
             Short format error: {}\n\
             Full format error: {}",
            short_err, full_err
        )),
    }
}

fn parse_full_signature(signature: &str) -> Result<()> {
    let signature_tokens: TokenStream2 = signature.parse()?;

    let file: File = parse2(signature_tokens).map_err(|e| {
        Error::new(
            Span::call_site(),
            format!("Failed to parse signature: {}", e),
        )
    })?;

    let item = file
        .items
        .into_iter()
        .next()
        .ok_or_else(|| Error::new(Span::call_site(), "No items found in parsed file"))?;

    let function = match item {
        SolidityItem::Function(func) => func,
        _ => {
            return Err(Error::new(
                Span::call_site(),
                "Parsed item is not a function",
            ))
        }
    };

    if function.name.is_none() {
        return Err(Error::new(Span::call_site(), "Function name is empty"));
    }

    eprintln!("signature is valid");

    Ok(())
}

fn validate_short_signature(signature: &str) -> core::result::Result<(), String> {
    let re =
        Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*\((([a-zA-Z][a-zA-Z0-9]*(\[[0-9]*\])?(,\s*)?)*)\)$;")
            .unwrap();
    if re.is_match(signature) {
        Ok(())
    } else {
        Err("Invalid short signature format".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{parse2, parse_quote, Attribute};

    #[test]
    fn test_function_id_calc() {
        const expected_signature: &'static str = "fvmWithdraw(uint8[])";
        const expected_func_id: [u8; 4] = [248u8, 47u8, 240u8, 224u8];

        let func_id = calculate_keccak256_bytes::<4>(&expected_signature);

        assert_eq!(func_id, expected_func_id);
    }

    #[test]
    fn test_parse_simple_signature() {
        let attr: Attribute = parse_quote! {
            #[signature("function simple() external")]
        };

        println!("Input Attribute: {:?}", attr);

        let parsed_sig = attr.parse_args::<SignatureAttribute>();
        match &parsed_sig {
            Ok(sig) => println!("Successfully parsed SignatureAttribute: {:?}", sig),
            Err(e) => println!("Failed to parse SignatureAttribute: {:?}", e),
        }
        assert!(
            parsed_sig.is_ok(),
            "Failed to parse SignatureAttribute: {:?}",
            parsed_sig.err()
        );

        let sig = parsed_sig.unwrap();
        assert_eq!(sig.signature, "function simple() external");
        assert!(sig.validate); // По умолчанию должно быть true
    }

    #[test]
    fn test_signature_parse_with_validate_false() {
        let attr: Attribute = parse_quote! {
            #[signature("function complex(uint256 x) external", false)]
        };

        println!("Input Attribute: {:?}", attr);

        let parsed_sig = attr.parse_args::<SignatureAttribute>();
        match &parsed_sig {
            Ok(sig) => println!("Successfully parsed SignatureAttribute: {:?}", sig),
            Err(e) => println!("Failed to parse SignatureAttribute: {:?}", e),
        }
        assert!(
            parsed_sig.is_ok(),
            "Failed to parse SignatureAttribute: {:?}",
            parsed_sig.err()
        );

        let sig = parsed_sig.unwrap();
        assert_eq!(sig.signature, "function complex(uint256 x) external");
        assert!(!sig.validate);
    }

    #[test]
    fn test_signature_parse_with_validate_true() {
        let attr: Attribute = parse_quote! {
            #[signature("function another(address a) external", true)]
        };

        let parsed_sig = attr.parse_args::<SignatureAttribute>();
        assert!(parsed_sig.is_ok());

        let sig = parsed_sig.unwrap();
        assert_eq!(sig.signature, "function another(address a) external");
        assert!(sig.validate);
    }

    #[test]
    fn test_route_parse_with_signature() {
        let input: TokenStream2 = quote! {
            #[signature("function fvm_deposit(bytes msg, address caller) external")]
            fn fvm_deposit(&mut self, msg: &[u8], caller: Address) {
                self.sdk.write(msg);
            }
        };

        let parsed_route = syn::parse2::<Route>(input);
        println!("{:?}", parsed_route);
        assert!(parsed_route.is_ok());
        let route = parsed_route.unwrap();
        assert_eq!(route.fn_name, "fvm_deposit");
        assert_eq!(route.args.len(), 2);
        assert_eq!(route.args[0].ident.to_string(), "msg");
        assert_eq!(route.args[1].ident.to_string(), "caller");
    }

    #[test]
    fn test_route_parse_with_validate_signature() {
        let input: TokenStream2 = quote! {
            #[signature("function fvm_withdraw(uint256 amount) external", true)]
            fn fvm_withdraw(&mut self, amount: u64) {
                self.sdk.write(&amount.to_le_bytes());
            }
        };

        println!("Input TokenStream2: {}", input);

        let parsed_route = parse2::<Route>(input);
        match &parsed_route {
            Ok(route) => println!("Successfully parsed Route: {:?}", route),
            Err(e) => println!("Failed to parse Route: {:?}", e),
        }
        assert!(
            parsed_route.is_ok(),
            "Failed to parse Route: {:?}",
            parsed_route.err()
        );

        let route = parsed_route.unwrap();
        assert_eq!(route.fn_name, "fvm_withdraw");
        assert_eq!(route.args.len(), 1);
        assert_eq!(route.args[0].ident.to_string(), "amount");
    }

    #[test]
    fn test_route_parse_with_function_id() {
        let input: TokenStream2 = quote! {
            #[function_id("0x12345678")]
            fn custom_function(&mut self, param: u32) {
                // Function body
            }
        };

        println!("Input TokenStream2: {}", input);

        let parsed_route = parse2::<Route>(input);
        match &parsed_route {
            Ok(route) => println!("Successfully parsed Route: {:?}", route),
            Err(e) => println!("Failed to parse Route: {:?}", e),
        }
        assert!(
            parsed_route.is_ok(),
            "Failed to parse Route: {:?}",
            parsed_route.err()
        );

        let route = parsed_route.unwrap();
        assert_eq!(route.fn_name, "custom_function");
        assert_eq!(route.function_id, [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(route.args.len(), 1);
        assert_eq!(route.args[0].ident.to_string(), "param");
    }

    #[test]
    fn test_parse_invalid_function_id_length() {
        let input = "0x1234";
        let result = parse_function_id(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_function_id_format() {
        let input = "12345678";
        let result = parse_function_id(input);
        assert!(result.is_err());
    }
    // TEST to tokens
    #[test]
    fn test_route_to_tokens() {
        let route = Route {
            function_id: [0x12, 0x34, 0x56, 0x78],
            fn_name: "my_function".to_string(),
            signature: "function my_function(uint256 x) external".to_string(),
            args: vec![Arg {
                ident: Ident::new("x", Span::call_site()),
                ty: parse_str::<Type>("u256").unwrap(),
            }],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [0x12, 0x34, 0x56, 0x78] => {
                let (x) = match my_functionCall::abi_decode(&input, true) {
                    Ok(decoded) => (decoded.x),
                    Err(_) => panic!("failed to decode input"),
                };
                let output = self.my_function(x).abi_encode();
                self.sdk.write(&output);
            }
        };

        assert_eq!(tokens.to_string(), expected_tokens.to_string());
    }

    #[test]
    fn test_route_to_tokens_multiple_args() {
        let route = Route {
            function_id: [0xde, 0xad, 0xbe, 0xef],
            fn_name: "multiple_args_function".to_string(),
            signature: "function multiple_args_function(uint256 x, address y) external".to_string(),
            args: vec![
                Arg {
                    ident: Ident::new("x", Span::call_site()),
                    ty: parse_str::<Type>("u256").unwrap(),
                },
                Arg {
                    ident: Ident::new("y", Span::call_site()),
                    ty: parse_str::<Type>("Address").unwrap(),
                },
            ],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [0xde, 0xad, 0xbe, 0xef] => {
                let (x, y) = match multiple_args_functionCall :: abi_decode (& input , true) {
                    Ok (decoded) => (decoded.x, decoded.y),
                    Err (_) => panic!("failed to decode input"),
                };
                let output = self.multiple_args_function(x, y).abi_encode();
                self.sdk.write(&output);
            }
        };

        assert_eq!(tokens.to_string(), expected_tokens.to_string());
    }

    #[test]
    fn test_route_to_tokens_with_references() {
        let route = Route {
            function_id: [0xab, 0xcd, 0xef, 0x12],
            fn_name: "function_with_references".to_string(),
            signature: "function function_with_references(uint256 x, address y) external"
                .to_string(),
            args: vec![
                Arg {
                    ident: Ident::new("x", Span::call_site()),
                    ty: parse_str::<Type>("&u256").unwrap(),
                },
                Arg {
                    ident: Ident::new("y", Span::call_site()),
                    ty: parse_str::<Type>("&mut Address").unwrap(),
                },
            ],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [0xab, 0xcd, 0xef, 0x12] => {
                let (x, y) = match function_with_referencesCall :: abi_decode (& input , true) {
                    Ok (decoded) => (decoded.x, decoded.y),
                    Err (_) => panic!("failed to decode input"),
                };
                let output = self.function_with_references(&x, &mut y).abi_encode(); // Обработка ссылок
                self.sdk.write(&output);
            }
        };

        assert_eq!(tokens.to_string(), expected_tokens.to_string());
    }
}

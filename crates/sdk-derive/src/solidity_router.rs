use crate::utils::{
    calculate_keccak256_bytes,
    get_raw_signature,
    parse_function_inputs,
    rust_name_to_sol,
    rust_type_to_sol,
};
use convert_case::Casing;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use regex::Regex;
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Attribute,
    Error,
    FnArg,
    Ident,
    ImplItem,
    ImplItemFn,
    Index,
    ItemImpl,
    ItemTrait,
    Lit,
    LitStr,
    Result,
    ReturnType,
    Token,
    TraitItem,
    TraitItemFn,
    Type,
};
use syn_solidity::{parse2, File, Item as SolidityItem};
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

    // Exclude fallback from the routes and create selectors for routing
    let routes_except_fallback: Vec<Route> = methods_to_route
        .iter()
        .filter(|r| r.fn_name != "fallback")
        .cloned()
        .collect();

    // Generate the main router code
    let router = generate_router(&ast, &routes_except_fallback, has_fallback);

    let codec_impl = routes_except_fallback
        .iter()
        .map(|r| r.generate_codec_impl())
        .collect::<Vec<_>>();

    let output = quote! {
        #ast

        #(#codec_impl);*

        // #router
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

                let mut full_input = fluentbase_sdk::alloc_slice(input_size as usize);

                self.sdk.read(&mut full_input, 0);
                let (selector, data_input) = full_input.split_at(4);

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
    pub return_args: Vec<Type>,
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
        let return_args = match &method.sig.output {
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple.elems.iter().cloned().collect(),
                _ => vec![(&**ty).clone()],
            },
            ReturnType::Default => vec![],
        };

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

        Ok(Route {
            function_id,
            fn_name: method.sig.ident.to_string(),
            signature: method_sol_signature,
            args,
            return_args,
            is_public,
        })
    }
}

impl Parse for SignatureAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let sig_str: LitStr = input.parse()?;
        let mut validate = false;

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
            let fn_name = Ident::new(
                &self.fn_name.to_case(convert_case::Case::Pascal),
                Span::call_site(),
            );

            let fn_call_name = Ident::new(&format!("{}Call", fn_name), Span::call_site());
            let decode_args: Vec<TokenStream2> =
                self.args.iter().map(|arg| arg.to_decode_token()).collect();

            let field_names: Vec<TokenStream2> = self
                .args
                .iter()
                .map(|arg| {
                    let ident = &arg.ident;
                    quote! { #ident }
                })
                .collect();

            quote! {
                // FIXME:
                // TODO: d1r1
                // abi_decode validates input function_id.
                // so we can't use our custom function_id here
                // we need to restore it to the #fn_call_name::SELECTOR
                input[0..4].copy_from_slice(&#fn_call_name::SELECTOR);
                let (#(#decode_args),*) = match #fn_call_name::abi_decode(&input, true) {
                    Ok(decoded) => (
                        #(decoded.#field_names),*
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

    pub fn generate_codec_impl(&self) -> TokenStream2 {
        let call_name = format_ident!("{}Call", self.fn_name.to_case(convert_case::Case::Pascal));
        let call_name_args = format_ident!("{}Args", call_name);
        let call_name_target = format_ident!("{}Target", call_name);

        let return_name =
            format_ident!("{}Return", self.fn_name.to_case(convert_case::Case::Pascal));
        let return_name_args = format_ident!("{}Args", return_name);
        let return_name_target = format_ident!("{}Target", return_name);

        let input_args = self.args.iter().map(|arg| &arg.ty).collect::<Vec<_>>();
        let call_args = quote! { (#(#input_args,)*) };

        let return_args = if self.return_args.is_empty() {
            quote! { () }
        } else {
            let return_types = &self.return_args;
            quote! { (#(#return_types,)*) }
        };

        let encode_input_fields = self.args.iter().enumerate().map(|(i, _)| {
            let index = Index::from(i);
            quote! { self.0.clone().#index }
        });

        let function_id = &self.function_id;
        let signature = &self.signature;
        let fn_name = format_ident!("{}", &self.fn_name);

        let input_params = self.args.iter().map(|arg| {
            let ty = &arg.ty;
            let ident = &arg.ident;
            quote! { #ident: #ty }
        });

        quote! {
            pub type #call_name_args = #call_args;
            pub struct #call_name(#call_name_args);
            impl #call_name {
                pub const SELECTOR: [u8; 4] = [#(#function_id,)*];
                pub const SIGNATURE: &'static str = #signature;

                pub fn new(args: #call_name_args) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> Bytes {
                    let mut buf = BytesMut::new();
                    SolidityABI::encode(&(#(#encode_input_fields,)*), &mut buf, 0).unwrap();
                    let encoded_args = buf.freeze();

                    Self::SELECTOR.iter().copied().chain(encoded_args).collect()
                }

                pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
                    let chunk = buf.chunk();
                    if chunk.len() < 4 {
                        return Err(CodecError::Decoding(
                            codec2::error::DecodingError::BufferTooSmall {
                                expected: 4,
                                found: chunk.len(),
                                msg: "buf too small to read fn selector".to_string(),
                            },
                        ));
                    }

                    let selector: [u8; 4] = chunk[..4].try_into().unwrap();
                    if selector != Self::SELECTOR {
                        return Err(CodecError::Decoding(
                            codec2::error::DecodingError::InvalidSelector {
                                expected: Self::SELECTOR,
                                found: selector,
                            },
                        ));
                    }

                    let args = SolidityABI::<#call_name_args>::decode(&&chunk[4..], 0)?;
                    Ok(Self(args))
                }
            }

            impl Deref for #call_name {
                type Target = #call_name_args;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            pub type #call_name_target = <#call_name as Deref>::Target;

            pub type #return_name_args = #return_args;
            pub struct #return_name(#return_name_args);

            impl #return_name {
                pub fn new(args: #return_name_args) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> Bytes {
                    let mut buf = BytesMut::new();
                    SolidityABI::encode(&self.0, &mut buf, 0).unwrap();
                    buf.freeze().into()
                }

                pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {
                    let args = SolidityABI::<#return_name_args>::decode(buf, 0)?;
                    Ok(Self(args))
                }
            }

            impl Deref for #return_name {
                type Target = #return_name_args;

                fn deref(&self) -> &Self::Target {
                    &self.0
                }
            }

            pub type #return_name_target = <#return_name as Deref>::Target;


        }
    }

    fn generate_tuple_type(&self) -> TokenStream2 {
        let type_tokens = self.args.iter().map(|arg| &arg.ty);
        quote! { (#(#type_tokens,)*) }
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
        match &self.ty {
            Type::Reference(ty_ref) if ty_ref.mutability.is_some() => {
                quote! { mut #ident }
            }
            _ => {
                quote! { #ident }
            }
        }
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

#[allow(dead_code)]
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

    let input_types: Vec<String> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                let ty = &pat_type.ty;
                Some(rust_type_to_sol(ty).to_string())
            } else {
                None
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

pub fn validate_solidity_signature(signature: &str) -> Result<()> {
    let short_result = validate_short_signature(signature);
    let full_result = parse_full_signature(signature);

    if short_result.is_ok() || full_result.is_ok() {
        Ok(())
    } else {
        let span = signature.span();
        let short_err = short_result.unwrap_err();
        let full_err = full_result.unwrap_err();

        Err(Error::new(
            span,
            format!(
                "Invalid signature. Neither short nor full format is valid.\n\
             Short format error: {}\n\
             Full format error: {}",
                short_err, full_err
            ),
        ))
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

    Ok(())
}

fn validate_short_signature(signature: &str) -> core::result::Result<(), String> {
    let signature_no_whitespace: String =
        signature.chars().filter(|c| !c.is_whitespace()).collect();

    let re = Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*\((([a-zA-Z][a-zA-Z0-9]*(\[[0-9]*\])?)(,([a-zA-Z][a-zA-Z0-9]*(\[[0-9]*\])?))*)?\)$").unwrap();
    if re.is_match(&signature_no_whitespace) {
        Ok(())
    } else {
        Err("Invalid short signature format".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{parse2, parse_quote, parse_str, Attribute};

    #[test]
    fn test_validate_short_signature() {
        // Valid signatures
        assert!(validate_short_signature("transfer(address,uint256)").is_ok());
        assert!(validate_short_signature("balanceOf(address)").is_ok());
        assert!(validate_short_signature("approve(address,uint256)").is_ok());
        assert!(validate_short_signature("transfer_tokens(address,uint256[])").is_ok());
        assert!(
            validate_short_signature("complex_function(uint256,bool,string,address[])").is_ok()
        );

        // Valid signatures with spaces
        assert!(validate_short_signature("transfer( address , uint256 )").is_ok());
        assert!(validate_short_signature(" balanceOf ( address ) ").is_ok());
        assert!(validate_short_signature("approve (address, uint256)").is_ok());
        assert!(validate_short_signature("transfer_tokens(address, uint256[])").is_ok());

        // Invalid signatures
        assert!(validate_short_signature("").is_err());
        assert!(validate_short_signature("invalid()signature").is_err());
        assert!(validate_short_signature("1invalidName(uint256)").is_err());
        assert!(validate_short_signature("transfer(address,uint256);").is_err());
        assert!(validate_short_signature("transfer()extra").is_err());
        assert!(validate_short_signature("transfer(uint256,)").is_err());
        assert!(validate_short_signature("transfer(uint256").is_err());

        // Error message check
        if let Err(msg) = validate_short_signature("invalid()signature") {
            assert_eq!(msg, "Invalid short signature format");
        } else {
            panic!("Expected an error, but got Ok");
        }
    }

    #[test]
    fn test_function_id_calc() {
        const EXPECTED_SIGNATURE: &'static str = "fvmWithdraw(uint8[])";
        const EXPECTED_FUNC_ID: [u8; 4] = [248u8, 47u8, 240u8, 224u8];

        let func_id = calculate_keccak256_bytes::<4>(&EXPECTED_SIGNATURE);

        assert_eq!(func_id, EXPECTED_FUNC_ID);
    }

    #[test]
    #[ignore]
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
        assert!(sig.validate);
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
            #[signature("function fvm_deposit(bytes msg, address caller) external;")]
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
            #[signature("function fvm_withdraw(uint256 amount) external;", true)]
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
        assert_eq!(route.function_id, [18u8, 52u8, 86u8, 120u8]);
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
            function_id: [18u8, 52u8, 86u8, 120u8],
            fn_name: "my_function".to_string(),
            signature: "function myFunction(uint256 x) external;".to_string(),
            args: vec![Arg {
                ident: Ident::new("x", Span::call_site()),
                ty: parse_str::<Type>("u256").unwrap(),
            }],
            return_args: vec![],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [18u8, 52u8, 86u8, 120u8] => {
                input[0..4].copy_from_slice(&myFunctionCall::SELECTOR);
                let (x) = match myFunctionCall::abi_decode(&input, true) {
                    Ok (decoded) => (decoded.x) ,
                    Err (_) => panic!("failed to decode input"),
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
            function_id: [222u8, 173u8, 190u8, 239u8],
            fn_name: "multiple_args_function".to_string(),
            signature: "function multiple_args_function(uint256 x, address y) external;"
                .to_string(),
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
            return_args: vec![],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [222u8, 173u8, 190u8, 239u8] => {
                input[0..4].copy_from_slice(&multipleArgsFunctionCall::SELECTOR);
                let (x, y) = match multipleArgsFunctionCall::abi_decode(&input, true) {
                    Ok(decoded) => (decoded.x, decoded.y),
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
            function_id: [171u8, 205u8, 239u8, 18u8],
            fn_name: "function_with_references".to_string(),
            signature: "function function_with_references(uint256 x, address y) external;"
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
            return_args: vec![],
            is_public: true,
        };

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        let expected_tokens = quote! {
            [171u8, 205u8, 239u8, 18u8] => {
                input[0..4].copy_from_slice(&functionWithReferencesCall::SELECTOR);
                let (x, mut y) = match functionWithReferencesCall::abi_decode(&input, true) {
                    Ok(decoded) => (decoded.x, decoded.y),
                    Err (_) => panic!("failed to decode input"),
                };
                let output = self.function_with_references(&x, &mut y).abi_encode();
                self.sdk.write(&output);
            }
        };

        assert_eq!(tokens.to_string(), expected_tokens.to_string());
    }
}

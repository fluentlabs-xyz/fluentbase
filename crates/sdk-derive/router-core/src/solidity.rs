use crate::utils::rust_type_to_sol;
use convert_case::{Case, Casing};
use darling::FromMeta;
use hex;
use proc_macro2::{Span, TokenStream as TokenStream2};
use proc_macro_error::abort;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute,
    Error,
    FnArg,
    Ident,
    ImplItem,
    ImplItemFn,
    Index,
    ItemImpl,
    LitStr,
    Result,
    ReturnType,
    Token,
    Type,
};

#[derive(Debug, FromMeta, Clone)]
pub struct FunctionIDAttribute {
    #[darling(default)]
    pub validate: Option<bool>,
    #[darling(skip)]
    pub function_id: Option<FunctionIDType>,
}

#[derive(Debug, Clone)]
enum FunctionIDType {
    Signature(String),
    HexString(String),
    ByteArray([u8; 4]),
}

impl FunctionIDAttribute {
    fn validate_signature(&self, signature: &str) -> Result<()> {
        if !signature.ends_with(")") || !signature.contains("(") {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("Invalid Solidity function signature: '{}'. Expected format: 'functionName(type1,type2,...)'", signature)
            ));
        }
        // TODO: d1r1 Add more detailed signature validation here
        Ok(())
    }

    pub fn function_id_hex(&self) -> Result<String> {
        self.function_id
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
            .and_then(|id| match id {
                FunctionIDType::Signature(sig) => {
                    use crypto_hashes::{digest::Digest, sha3::Keccak256};
                    let mut hash = Keccak256::new();
                    hash.update(sig.as_bytes());
                    let mut dst = [0u8; 4];
                    dst.copy_from_slice(&hash.finalize()[0..4]);
                    Ok(format!("0x{}", hex::encode(&dst[..4])))
                }
                FunctionIDType::HexString(hex_str) => Ok(hex_str.clone()),
                FunctionIDType::ByteArray(arr) => Ok(format!("0x{}", hex::encode(arr))),
            })
    }

    pub fn function_id_bytes(&self) -> Result<[u8; 4]> {
        self.function_id
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
            .and_then(|id| match id {
                FunctionIDType::Signature(sig) => {
                    use crypto_hashes::{digest::Digest, sha3::Keccak256};
                    let mut hash = Keccak256::new();
                    hash.update(sig.as_bytes());
                    let mut dst = [0u8; 4];
                    dst.copy_from_slice(&hash.finalize()[0..4]);
                    Ok(dst)
                }
                FunctionIDType::HexString(hex_str) => {
                    let bytes = hex::decode(hex_str.trim_start_matches("0x")).map_err(|e| {
                        syn::Error::new(Span::call_site(), format!("Invalid hex string: {}", e))
                    })?;
                    if bytes.len() != 4 {
                        Err(syn::Error::new(
                            Span::call_site(),
                            format!(
                                "Invalid hex string length. Expected 4 bytes, found {}",
                                bytes.len()
                            ),
                        ))
                    } else {
                        let mut arr = [0u8; 4];
                        arr.copy_from_slice(&bytes);
                        Ok(arr)
                    }
                }
                FunctionIDType::ByteArray(arr) => Ok(*arr),
            })
    }

    pub fn signature(&self) -> Option<String> {
        self.function_id.as_ref().and_then(|id| match id {
            FunctionIDType::Signature(sig) => Some(sig.clone()),
            _ => None,
        })
    }
}

impl Parse for FunctionIDAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let lookahead = input.lookahead1();
        let function_id = if lookahead.peek(LitStr) {
            let lit_str: LitStr = input.parse()?;
            let value = lit_str.value();
            if value.starts_with("0x") && value.len() == 10 {
                Some(FunctionIDType::HexString(value))
            } else if value.contains('(') && value.ends_with(')') {
                Some(FunctionIDType::Signature(value))
            } else {
                return Err(syn::Error::new(
                    lit_str.span(),
                    "Invalid function ID format. Expected either a Solidity function signature (e.g., 'transfer(address,uint256)') or a 4-byte hex string (e.g., '0x12345678')"
                ));
            }
        } else if lookahead.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);
            let bytes: Vec<u8> = content
                .parse_terminated(syn::Lit::parse, Token![,])?
                .into_iter()
                .map(|lit| match lit {
                    syn::Lit::Int(lit_int) => lit_int.base10_parse::<u8>().map_err(|_| {
                        syn::Error::new_spanned(
                            &lit_int,
                            "Invalid byte value. Expected an integer between 0 and 255",
                        )
                    }),
                    _ => Err(syn::Error::new_spanned(&lit, "Expected u8 literal (0-255)")),
                })
                .collect::<Result<_>>()?;
            if bytes.len() != 4 {
                return Err(syn::Error::new(
                    content.span(),
                    format!(
                        "Invalid byte array length. Expected exactly 4 bytes, found {}",
                        bytes.len()
                    ),
                ));
            }
            Some(FunctionIDType::ByteArray(bytes.try_into().unwrap()))
        } else {
            return Err(syn::Error::new(
                input.span(),
                "Expected a string literal for function signature or hex string, or a byte array [u8; 4]"
            ));
        };

        let mut validate = None;
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            let meta = input.parse::<syn::Meta>()?;
            match meta {
                syn::Meta::List(list) => {
                    if list.path.is_ident("validate") {
                        let nested = list
                            .parse_args_with(Punctuated::<syn::Expr, Token![,]>::parse_terminated)
                            .map_err(|e|
                                syn::Error::new(
                                    list.span(),
                                    format!("Invalid 'validate' attribute: {}. Expected 'validate(true)' or 'validate(false)'", e)
                                )
                            )?;
                        if nested.len() != 1 {
                            return Err(syn::Error::new(
                                list.span(),
                                format!("Expected exactly one argument for 'validate', found {}", nested.len())
                            ));
                        }
                        if let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Bool(lit_bool),
                            ..
                        }) = &nested[0]
                        {
                            validate = Some(lit_bool.value);
                        } else {
                            return Err(syn::Error::new(
                                nested[0].span(),
                                "Expected a boolean literal (true or false) for 'validate'"
                            ));
                        }
                    } else {
                        return Err(syn::Error::new(list.span(), "Unexpected attribute. Only 'validate' is supported"));
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        meta.span(),
                        "Expected 'validate' attribute in the format: validate(true) or validate(false)"
                    ))
                }
            }
        }

        Ok(FunctionIDAttribute {
            validate,
            function_id,
        })
    }
}

impl ToTokens for FunctionIDAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let validate = self.validate.unwrap_or(true);

        if let Some(function_id) = &self.function_id {
            let function_id_hex = match self.function_id_hex() {
                Ok(hex) => hex,
                Err(e) => {
                    tokens.extend(error_tokens(e));
                    return;
                }
            };
            let function_id_bytes = match self.function_id_bytes() {
                Ok(bytes) => bytes,
                Err(e) => {
                    tokens.extend(error_tokens(e));
                    return;
                }
            };

            if let FunctionIDType::Signature(sig) = function_id {
                if validate {
                    if let Err(e) = self.validate_signature(sig) {
                        tokens.extend(error_tokens(e));
                        return;
                    }
                }

                tokens.extend(quote! {
                    const FUNCTION_SIGNATURE: &str = #sig;
                });
            }

            tokens.extend(quote! {
                const FUNCTION_ID_HEX: &str = #function_id_hex;
                const FUNCTION_ID_BYTES: [u8; 4] = [#(#function_id_bytes),*];
            });
        } else {
            tokens.extend(quote! {
                compile_error!("FunctionID attribute requires a value. Use either a Solidity function signature, a 4-byte hex string, or a byte array [u8; 4]");
            });
        }
    }
}

fn error_tokens(e: syn::Error) -> TokenStream2 {
    let error_msg = e.to_string();
    quote! { compile_error!(#error_msg); }
}

struct SolidityRouter {
    ast: ItemImpl,
    routes: Vec<Route>,
    has_fallback: bool,
}

impl SolidityRouter {
    fn generate_router(&self) -> TokenStream2 {
        eprintln!("op SolidityRouter generate_router");
        let struct_name = &self.ast.self_ty;
        let generics = &self.ast.generics;

        let routes_except_fallback: Vec<&Route> = self
            .routes
            .iter()
            .filter(|r| r.fn_name != "fallback")
            .collect();

        eprintln!("routes len: {:?}", self.routes.len());
        eprintln!(
            "routes_except_fallback len: {:?}",
            routes_except_fallback.len()
        );

        // Generate token streams for each route
        let routes_tokens: Vec<TokenStream2> = routes_except_fallback
            .iter()
            .map(|route| route.to_token_stream())
            .collect();

        // Add fallback handling
        let fallback_arm = if self.has_fallback {
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
        let match_arms = if routes_except_fallback.is_empty() {
            quote! {
                _ => panic!("No methods to route :("),
            }
        } else {
            quote! {
                #(#routes_tokens),*,
                #fallback_arm
            }
        };

        let input_size_check = if self.has_fallback {
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

                    match [selector[0], selector[1], selector[2], selector[3]] {
                        #match_arms
                    }
                }
            }
        }
    }
}

impl Parse for SolidityRouter {
    fn parse(input: ParseStream) -> Result<Self> {
        let ast: ItemImpl = input.parse()?;

        // Parse all routes from the provided implementation
        let routes = parse_all_methods(&ast)?;

        let methods_to_route: Vec<Route> = if ast.trait_.is_some() {
            routes.clone()
        } else {
            routes
                .into_iter()
                .filter(|r| r.is_public && r.fn_name != "deploy")
                .collect()
        };

        let has_fallback = methods_to_route.iter().any(|m| m.fn_name == "fallback");

        Ok(SolidityRouter {
            ast,
            routes: methods_to_route,
            has_fallback,
        })
    }
}

impl ToTokens for SolidityRouter {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        eprintln!("op SolidityRouter to_tokens");
        let ast = &self.ast;
        eprintln!("routes len: {:?}", self.routes.len());

        let routes_except_fallback: Vec<&Route> = self
            .routes
            .iter()
            .filter(|r| r.fn_name != "fallback")
            .collect();

        eprintln!(
            "routes_except_fallback len: {:?}",
            routes_except_fallback.len()
        );

        let codec_impl = routes_except_fallback
            .iter()
            .map(|r| r.generate_codec_impl())
            .collect::<Vec<_>>();

        let router = &self.generate_router();

        tokens.extend(quote! {
            #ast

            #(#codec_impl)*

            #router
        });
    }
}

pub fn derive_solidity_router(input: TokenStream2) -> Result<TokenStream2> {
    eprintln!("op derive_solidity_router");
    let router: SolidityRouter = syn::parse2(input)?;
    Ok(quote!(#router).into())
}

#[derive(Clone, Debug)]
struct Route {
    pub function_id_attr: Option<FunctionIDAttribute>,
    // function_id calculated from method
    pub function_id: [u8; 4],
    // fn_name in camelCase
    pub fn_name: String,
    // short form of signature derived from method
    pub signature: String,
    pub args: Vec<Arg>,
    pub return_args: Vec<Type>,
    pub original_fn: syn::ImplItemFn,
    pub is_public: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Arg {
    pub ty: Type,
    pub ident: Ident,
}

impl Route {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let function_id_attr = attrs
            .iter()
            .find(|attr| attr.path().is_ident("function_id"))
            .map(|attr| attr.parse_args::<FunctionIDAttribute>())
            .transpose()?;

        let original_fn: ImplItemFn = input.parse()?;

        let args: Vec<Arg> = Arg::from(&original_fn);
        let return_args = match &original_fn.sig.output {
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple.elems.iter().cloned().collect(),
                _ => vec![(&**ty).clone()],
            },
            ReturnType::Default => vec![],
        };

        let fn_name = to_camel_case(&original_fn.sig.ident);
        let method_sol_signature = get_sol_signature(
            &fn_name.to_string(),
            &args
                .iter()
                .map(|a| {
                    let ty = &a.ty;
                    rust_type_to_sol(ty).to_string()
                })
                .collect::<Vec<String>>(),
        );

        let calculated_function_id = keccak256(&method_sol_signature);
        let (function_id, signature) = match &function_id_attr {
            Some(attr) if attr.validate.unwrap_or(true) => {
                let attr_function_id = attr.function_id_bytes()?;
                if attr_function_id != calculated_function_id {
                    return Err(create_mismatch_error(
                        attr,
                        &method_sol_signature,
                        calculated_function_id,
                    ));
                }
                (
                    attr_function_id,
                    attr.signature().unwrap_or(method_sol_signature.clone()),
                )
            }
            Some(attr) => (
                attr.function_id_bytes()?,
                attr.signature().unwrap_or(method_sol_signature.clone()),
            ),
            None => (calculated_function_id, method_sol_signature.clone()),
        };

        let is_public = matches!(original_fn.vis, syn::Visibility::Public(_));

        Ok(Route {
            function_id_attr,
            function_id,
            fn_name: fn_name.to_string(),
            signature: method_sol_signature,
            args,
            return_args,
            original_fn,
            is_public,
        })
    }

    fn to_token_stream(&self) -> TokenStream2 {
        let fn_name = &self.original_fn.sig.ident;
        let fn_call = format_ident!("{}Call", &self.fn_name.to_case(Case::Pascal));
        let fn_call_selector = quote! { #fn_call::SELECTOR };
        let fn_call_args_ty = format_ident!("{}Args", fn_call);

        let fn_return = format_ident!("{}Return", &self.fn_name.to_case(Case::Pascal));

        let arg_names: Vec<TokenStream2> =
            self.args.iter().map(|arg| arg.to_decode_token()).collect();
        let field_indices: Vec<TokenStream2> = self
            .args
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let index = Index::from(i);
                quote! { #index }
            })
            .collect();

        let fn_call_args = self.get_function_call_args();

        quote! {
        #fn_call_selector => {
            let (#(#arg_names),*) = match SolidityABI::<#fn_call_args_ty>::decode(
                &data_input, 0
            ) {
                Ok(decoded) => (#(decoded.#field_indices),*),
                Err(err) => {
                    panic!("failed to decode input: {:?}", err);
                }
            };
            let output = self.#fn_name(#fn_call_args);
            let encoded_output = #fn_return::new((output,)).encode();

            self.sdk.write(&encoded_output);

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
            pub use codec2::encoder::Encoder;
            pub type #call_name_args = #call_args;
            pub struct #call_name(#call_name_args);
            impl #call_name {
                // If the function id is provided, it is calculated from the rust function signature
                // Otherwise using the given function id
                pub const SELECTOR: [u8; 4] = [#(#function_id,)*];
                // SIGNATURE always derives from the rust function
                pub const SIGNATURE: &'static str = #signature;

                pub fn new(args: #call_name_args) -> Self {
                    Self(args)
                }

                pub fn encode(&self) -> Bytes {
                    let mut buf = BytesMut::new();
                    SolidityABI::encode(&(#(#encode_input_fields,)*), &mut buf, 0).unwrap();
                    let encoded_args = buf.freeze();
                    // let clean_args = if #call_name_args::IS_DYNAMIC {
                    //     encoded_args[32..].to_vec()
                    // } else {
                    //     encoded_args.to_vec()
                    // };

                    // let clean_args = if codec2::encoder::is_dynamic::<SolidityABI<#call_name_args>>() {
                    //     encoded_args[32..].to_vec()
                    // } else {
                    //     encoded_args.to_vec()
                    // };

                    let clean_args = if SolidityABI::<GreetingCallArgs>::is_dynamic() {
                        encoded_args[32..].to_vec()
                    } else {
                        encoded_args.to_vec()
                    };

                    Self::SELECTOR.iter().copied().chain(clean_args).collect()
                }

                pub fn decode(buf: &impl Buf) -> Result<Self, CodecError> {

                    let dynamic_offset = if SolidityABI::<GreetingCallArgs>::is_dynamic() {
                        fluentbase_sdk::U256::from(32).to_be_bytes()
                    } else {
                        []
                    };
                    let chunk = dynamic_offset.chain(&buf.chunk());

                    let args = SolidityABI::<#call_name_args>::decode(&&chunk, 0)?;
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

impl Parse for Route {
    fn parse(input: ParseStream) -> Result<Self> {
        Route::parse(input)
    }
}

impl ToTokens for Route {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let mut ts = self.to_token_stream();
        tokens.extend(ts);
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

pub fn to_camel_case(ident: &Ident) -> Ident {
    let span = ident.span();
    let camel_name = ident.to_string().to_case(Case::Camel);
    Ident::new(&camel_name, span)
}

pub fn keccak256(signature: &str) -> [u8; 4] {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    Keccak256::digest(signature.as_bytes())
        .as_slice()
        .get(..4)
        .unwrap_or(&[0; 4])
        .try_into()
        .unwrap_or([0; 4])
}

pub fn get_sol_signature(fn_name: &str, args: &[String]) -> String {
    format!("{}({})", fn_name, args.join(","))
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

fn create_mismatch_error(
    attr: &FunctionIDAttribute,
    method_signature: &str,
    calculated_id: [u8; 4],
) -> syn::Error {
    let error_msg = match &attr.function_id {
        Some(FunctionIDType::Signature(attr_signature)) => format!(
            "Signature mismatch. Attribute: '{}' (id: 0x{:02x}{:02x}{:02x}{:02x}). Method: '{}' (id: 0x{:02x}{:02x}{:02x}{:02x}).",
            attr_signature, attr.function_id_bytes().unwrap()[0], attr.function_id_bytes().unwrap()[1], attr.function_id_bytes().unwrap()[2], attr.function_id_bytes().unwrap()[3],
            method_signature, calculated_id[0], calculated_id[1], calculated_id[2], calculated_id[3]
        ),
        _ => format!(
            "Function ID mismatch. Attribute: 0x{:02x}{:02x}{:02x}{:02x}. Calculated: 0x{:02x}{:02x}{:02x}{:02x}.",
            attr.function_id_bytes().unwrap()[0], attr.function_id_bytes().unwrap()[1], attr.function_id_bytes().unwrap()[2], attr.function_id_bytes().unwrap()[3],
            calculated_id[0], calculated_id[1], calculated_id[2], calculated_id[3]
        ),
    };
    syn::Error::new(Span::call_site(), error_msg)
}

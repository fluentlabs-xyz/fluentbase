use crate::{
    codec::CodecGenerator,
    function_id::FunctionIDAttribute,
    mode::RouterMode,
    utils::rust_type_to_sol,
};
use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use serde_json::{json, Value};
use syn::{
    parse::{Parse, ParseStream},
    parse_quote,
    Attribute,
    FnArg,
    Ident,
    ImplItemFn,
    Index,
    Result,
    ReturnType,
    TraitItemFn,
    Type,
};

/// Represents a routable method with its metadata and implementation details.
#[derive(Clone, Debug)]
pub struct Route {
    /// Optional function identifier attribute for custom routing
    pub function_id_attr: Option<FunctionIDAttribute>,
    /// Computed 4-byte function selector
    pub function_id: [u8; 4],
    /// Method name in camelCase format
    pub fn_name: String,
    /// Solidity-compatible function signature
    pub signature: String,
    /// Method parameters
    pub args: Vec<MethodParameter>,
    /// Return type parameters
    pub return_types: Vec<Type>,
    /// Original function implementation
    pub original_fn: MethodType,
    /// Indicates if the method is publicly accessible
    pub is_public: bool,
}

#[derive(Clone, Debug)]
pub enum MethodType {
    Impl(ImplItemFn),
    Trait(TraitItemFn),
}

/// Represents a method parameter with its type and identifier.
#[derive(Clone, Debug, PartialEq)]
pub struct MethodParameter {
    /// Original parameter type including references
    pub original_ty: Type,
    /// Storage parameter type (owned version)
    pub ty: Type,
    /// Parameter identifier
    pub ident: Ident,
}

impl Route {
    /// Creates a new Route instance by parsing the implementation method.
    ///
    /// # Arguments
    /// * `method_impl` - The function implementation to parse
    ///
    /// # Returns
    /// * `Result<Route>` - Parsed route or parsing error
    pub fn new(method_impl: &ImplItemFn) -> Result<Self> {
        let parameters = MethodParameter::from_impl(method_impl);
        let return_types = Self::extract_return_types(&method_impl.sig.output);
        let fn_name = str_to_camel_case(&method_impl.sig.ident.to_string());

        let parameter_types = Self::get_param_types(&parameters, &fn_name)?;
        let method_signature = Self::generate_signature(&fn_name, &parameter_types);

        Ok(Route {
            function_id_attr: None, // Will be set later
            function_id: [0; 4],    // Will be computed later
            fn_name,
            signature: method_signature,
            args: parameters,
            return_types,
            original_fn: MethodType::Impl(method_impl.clone()),
            is_public: matches!(method_impl.vis, syn::Visibility::Public(_)),
        })
    }

    pub fn process_function_id(
        method_signature: &str,
        function_id_attr: Option<FunctionIDAttribute>,
    ) -> Result<([u8; 4], String)> {
        let function_id = if let Some(attr) = &function_id_attr {
            let attr_function_id = attr.function_id_bytes()?;
            if attr.validate.unwrap_or(true) && attr_function_id != keccak256(method_signature) {
                return Err(create_mismatch_error(
                    attr,
                    method_signature,
                    keccak256(method_signature),
                ));
            }
            attr_function_id
        } else {
            keccak256(method_signature)
        };

        let signature = function_id_attr
            .and_then(|attr| attr.signature())
            .unwrap_or_else(|| method_signature.to_string());

        Ok((function_id, signature))
    }

    fn get_param_types(params: &[MethodParameter], fn_name: &str) -> Result<Vec<String>> {
        params
            .iter()
            .map(|param| {
                rust_type_to_sol(&param.ty)
                    .map(|tokens| tokens.to_string())
                    .map_err(|e| {
                        syn::Error::new(
                            Span::call_site(),
                            format!(
                                "Failed to parse parameter type in function '{}': {}",
                                fn_name, e
                            ),
                        )
                    })
            })
            .collect::<Result<Vec<String>>>()
    }

    /// Returns the signature of the method
    pub fn sig(&self) -> &syn::Signature {
        match &self.original_fn {
            MethodType::Impl(m) => &m.sig,
            MethodType::Trait(m) => &m.sig,
        }
    }

    /// Extracts return types from the method signature.
    fn extract_return_types(return_type: &ReturnType) -> Vec<Type> {
        match return_type {
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple.elems.iter().cloned().collect(),
                _ => vec![(&**ty).clone()],
            },
            ReturnType::Default => vec![],
        }
    }

    /// Generates a Solidity-compatible function signature.
    fn generate_signature(name: &str, param_types: &[String]) -> String {
        format!("{}({})", name, param_types.join(","))
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    pub fn generate_codec_impl(&self, mode: &RouterMode) -> TokenStream2 {
        let input_types: Vec<&Type> = self.args.iter().map(|arg| &arg.ty).collect();
        let return_types: Vec<&Type> = self.return_types.iter().collect();

        CodecGenerator::new(
            &self.fn_name,
            &self.function_id,
            &self.signature,
            input_types,
            return_types,
            mode,
        )
        .generate()
    }
}

impl TryFrom<&TraitItemFn> for Route {
    type Error = syn::Error;

    fn try_from(method: &TraitItemFn) -> Result<Self> {
        let parameters = MethodParameter::from_trait_fn(method);
        let return_types = Self::extract_return_types(&method.sig.output);
        let fn_name = str_to_camel_case(&method.sig.ident.to_string());

        let parameter_types = Self::get_param_types(&parameters, &fn_name)?;
        let method_signature = Self::generate_signature(&fn_name, &parameter_types);

        let function_id_attr = method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("function_id"))
            .map(|attr| attr.parse_args::<FunctionIDAttribute>())
            .transpose()?;

        let (function_id, signature) =
            Self::process_function_id(&method_signature, function_id_attr.clone())?;

        Ok(Self {
            function_id_attr,
            function_id,
            fn_name,
            signature,
            args: parameters,
            return_types,
            original_fn: MethodType::Trait(method.clone()),
            is_public: true,
        })
    }
}

impl Parse for Route {
    /// Parses a Route from the input stream.
    ///
    /// # Arguments
    /// * `input` - Input token stream
    ///
    /// # Returns
    /// * `Result<Route>` - Parsed route or parsing error
    fn parse(input: ParseStream) -> Result<Self> {
        // Parse attributes to find function_id
        let attrs = input.call(Attribute::parse_outer)?;
        let function_id_attr = attrs
            .iter()
            .find(|attr| attr.path().is_ident("function_id"))
            .map(|attr| attr.parse_args::<FunctionIDAttribute>())
            .transpose()?;

        // Parse the function implementation
        let method_impl: ImplItemFn = input.parse()?;
        let mut route = Self::new(&method_impl)?;

        // Calculate function ID based on attribute or signature
        let method_signature = route.signature.clone();
        let (function_id, signature) =
            Self::process_function_id(&method_signature, function_id_attr.clone())?;

        route.function_id_attr = function_id_attr;
        route.function_id = function_id;
        route.signature = signature;

        Ok(route)
    }
}

impl ToTokens for Route {
    /// Converts the route to a token stream.
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let fn_name = match &self.original_fn {
            MethodType::Impl(m) => &m.sig.ident,
            MethodType::Trait(m) => &m.sig.ident,
        };
        let fn_call = format_ident!("{}Call", &self.fn_name.to_case(Case::Pascal));
        let fn_call_selector = quote! { #fn_call::SELECTOR };
        let fn_return = format_ident!("{}Return", &self.fn_name.to_case(Case::Pascal));

        // Generate parameter decoding tokens
        let param_decoders = self.args.iter().map(|param| param.to_decode_token());
        let param_indices = (0..self.args.len()).map(|i| Index::from(i));
        let fn_call_params = self.get_function_call_params();

        // Generate the method dispatch code
        let dispatch_code = match self.return_types.len() {
            0 => quote! {
                #fn_call_selector => {
                    let (#(#param_decoders),*) = match #fn_call::decode(&params) {
                        Ok(decoded) => (#(decoded.0.#param_indices),*),
                        Err(err) => panic!("Failed to decode input parameters: {:?}", err),
                    };

                    let output = self.#fn_name(#fn_call_params);

                    // If output is unit type (), do not wrap it in a tuple
                    let encoded_output = [0u8; 0];

                    self.sdk.write(&encoded_output);
                }
            },
            1 => quote! {
                #fn_call_selector => {
                    let (#(#param_decoders),*) = match #fn_call::decode(&params) {
                        Ok(decoded) => (#(decoded.0.#param_indices),*),
                        Err(err) => panic!("Failed to decode input parameters: {:?}", err),
                    };

                    let output = self.#fn_name(#fn_call_params);

                    let encoded_output = #fn_return::new((output,)).encode();

                    self.sdk.write(&encoded_output);
                }
            },
            _ => quote! {
                    #fn_call_selector => {
                        let (#(#param_decoders),*) = match #fn_call::decode(&params) {
                            Ok(decoded) => (#(decoded.0.#param_indices),*),
                            Err(err) => panic!("Failed to decode input parameters: {:?}", err),
                        };

                        let output = self.#fn_name(#fn_call_params);

                        let encoded_output = #fn_return::new(output).encode();

                        self.sdk.write(&encoded_output);
                    }
            },
        };

        tokens.extend(dispatch_code);
    }
}

impl Route {
    /// Generates function call parameters with proper reference handling.
    fn get_function_call_params(&self) -> TokenStream2 {
        let params = self.args.iter().map(|param| param.to_call_token());
        quote! { #(#params),* }
    }

    /// Generates ABI JSON for the method
    pub fn to_abi_json(&self) -> Value {
        // Create inputs array
        let inputs = self
            .args
            .iter()
            .enumerate()
            .map(|(i, param)| {
                let sol_type = rust_type_to_sol(&param.ty)
                    .map(|t| t.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                json!({
                    "name": param.ident.to_string(),
                    "type": sol_type.trim(),
                    "internalType": sol_type.trim()
                })
            })
            .collect::<Vec<Value>>();

        // Create outputs array
        let outputs = self
            .return_types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                let sol_type = rust_type_to_sol(ty)
                    .map(|t| t.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                json!({
                    "name": format!("return_{}", i),
                    "type": sol_type.trim(),
                    "internalType": sol_type.trim()
                })
            })
            .collect::<Vec<Value>>();

        // Determine state mutability
        let state_mutability = match &self.original_fn {
            MethodType::Impl(m) => {
                if m.sig.inputs.iter().any(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        if let Type::Reference(type_ref) = &*pat_type.ty {
                            return type_ref.mutability.is_some();
                        }
                    }
                    false
                }) {
                    "nonpayable"
                } else {
                    "view"
                }
            }
            MethodType::Trait(_) => "nonpayable", // Default for trait methods
        };

        // Create the function ABI entry
        json!({
            "name": self.fn_name,
            "type": "function",
            "inputs": inputs,
            "outputs": outputs,
            "stateMutability": state_mutability
        })
    }
}

impl MethodParameter {
    /// Creates method parameters from a function implementation.
    fn from_impl(method: &ImplItemFn) -> Vec<Self> {
        method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let FnArg::Typed(pat_type) = arg {
                    if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        let original_ty = (*pat_type.ty).clone();
                        let ty = Self::to_storage_type(&original_ty);
                        Some(MethodParameter {
                            original_ty,
                            ty,
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

    pub fn from_trait_fn(method: &TraitItemFn) -> Vec<Self> {
        method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let FnArg::Typed(pat_type) = arg {
                    if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        let original_ty = (*pat_type.ty).clone();
                        let ty = Self::to_storage_type(&original_ty);
                        Some(MethodParameter {
                            original_ty,
                            ty,
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

    /// Converts reference types to their owned versions
    fn to_storage_type(ty: &Type) -> Type {
        match ty {
            Type::Reference(type_ref) => match &*type_ref.elem {
                Type::Slice(slice) if is_u8_type(&slice.elem) => {
                    parse_quote!(::alloc::vec::Vec<u8>)
                }
                Type::Path(path) if is_str_type(path) => {
                    parse_quote!(::alloc::string::String)
                }
                other => (*other).clone(),
            },
            other => other.clone(),
        }
    }

    /// Generates a token for parameter decoding.
    fn to_decode_token(&self) -> TokenStream2 {
        let ident = &self.ident;
        match &self.original_ty {
            Type::Reference(ty_ref) if ty_ref.mutability.is_some() => {
                quote! { mut #ident }
            }
            _ => quote! { #ident },
        }
    }

    /// Generates a token for function call parameter.
    fn to_call_token(&self) -> TokenStream2 {
        let ident = &self.ident;
        match &self.original_ty {
            Type::Reference(ty_ref) if ty_ref.mutability.is_some() => {
                quote! { &mut #ident }
            }
            Type::Reference(_) => {
                quote! { &#ident }
            }
            _ => quote! { #ident },
        }
    }
}

fn is_u8_type(ty: &Type) -> bool {
    if let Type::Path(path) = ty {
        path.path
            .segments
            .last()
            .map(|seg| seg.ident == "u8")
            .unwrap_or(false)
    } else {
        false
    }
}

fn is_str_type(path: &syn::TypePath) -> bool {
    path.path
        .segments
        .last()
        .map(|seg| seg.ident == "str")
        .unwrap_or(false)
}

/// Creates an error for function ID mismatch.
fn create_mismatch_error(
    attr: &FunctionIDAttribute,
    method_signature: &str,
    method_fn_id: [u8; 4],
) -> syn::Error {
    let mut message = format!(
        "Function ID mismatch.\nMethod signature: '{}'\n",
        method_signature
    );

    let attr_fn_id = attr.function_id_bytes().unwrap_or([0; 4]);

    message.push_str(&format!(
        "Expected function ID: 0x{} {:?}\nCalculated function ID: 0x{} {:?}",
        hex::encode(attr_fn_id),
        attr_fn_id,
        hex::encode(method_fn_id),
        method_fn_id,
    ));
    syn::Error::new(Span::call_site(), message)
}

pub fn ident_to_camel_case(ident: &Ident) -> Ident {
    let span = ident.span();
    let camel_name = ident.to_string().to_case(Case::Camel);
    Ident::new(&camel_name, span)
}

pub fn str_to_camel_case(s: &str) -> String {
    s.to_case(Case::Camel)
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

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use proc_macro2::TokenStream as TokenStream2;
    use syn::parse_quote;

    #[test]
    fn test_route_to_tokens() {
        let trait_method: TraitItemFn = parse_quote! {
            fn transfer(&self, to: String, amount: u64) -> bool;
        };

        let route =
            Route::try_from(&trait_method).expect("Failed to convert trait method to Route");

        let mut tokens = TokenStream2::new();
        route.to_tokens(&mut tokens);

        assert_snapshot!("route_expansion", tokens.to_string());
    }
}

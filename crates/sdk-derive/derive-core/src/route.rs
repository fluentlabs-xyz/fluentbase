use crate::{
    codec::CodecGenerator,
    function_id::FunctionIDAttribute,
    mode::RouterMode,
    utils::rust_type_to_sol,
};
use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
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
pub(crate) struct MethodParameter {
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

        let parameter_types = parameters
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
            .collect::<Result<Vec<String>>>()?;

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

    fn from_trait_fn(method: &TraitItemFn) -> Result<Self> {
        let parameters = MethodParameter::from_trait_fn(method);
        let return_types = Self::extract_return_types(&method.sig.output);
        let fn_name = str_to_camel_case(&method.sig.ident.to_string());

        let parameter_types = parameters
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
            .collect::<Result<Vec<String>>>()?;

        let method_signature = Self::generate_signature(&fn_name, &parameter_types);

        // Find function_id attribute if present
        let function_id_attr = method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("function_id"))
            .map(|attr| attr.parse_args::<FunctionIDAttribute>())
            .transpose()?;

        // Calculate function ID based on attribute or signature

        let (function_id, signature) = match &function_id_attr {
            Some(attr) if attr.validate.unwrap_or(true) => {
                let attr_function_id = attr.function_id_bytes()?;
                let calculated_id = keccak256(&method_signature);

                if attr_function_id != calculated_id {
                    return Err(create_mismatch_error(
                        attr,
                        &method_signature,
                        calculated_id,
                    ));
                }
                (
                    attr_function_id,
                    attr.signature().unwrap_or(method_signature),
                )
            }
            Some(attr) => (
                attr.function_id_bytes()?,
                attr.signature().unwrap_or(method_signature),
            ),
            None => (keccak256(&method_signature), method_signature),
        };

        Ok(Route {
            function_id_attr,
            function_id,
            fn_name,
            signature,
            args: parameters,
            return_types,
            original_fn: MethodType::Trait(method.clone()),
            is_public: true, // trait methods are always public
        })
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
        let (function_id, signature) = match &function_id_attr {
            Some(attr) if attr.validate.unwrap_or(true) => {
                let attr_function_id = attr.function_id_bytes()?;
                let calculated_id = keccak256(&method_signature);

                if attr_function_id != calculated_id {
                    return Err(create_mismatch_error(
                        attr,
                        &method_signature,
                        calculated_id,
                    ));
                }
                (
                    attr_function_id,
                    attr.signature().unwrap_or(method_signature),
                )
            }
            Some(attr) => (
                attr.function_id_bytes()?,
                attr.signature().unwrap_or(method_signature),
            ),
            None => (keccak256(&method_signature), method_signature),
        };

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
        let fn_call_args = format_ident!("{}Args", fn_call);
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

    fn from_trait_fn(method: &TraitItemFn) -> Vec<Self> {
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
    signature: &str,
    calculated_id: [u8; 4],
) -> syn::Error {
    syn::Error::new(
        Span::call_site(),
        format!(
            "Function ID mismatch for signature '{}'. \
             Expected {:?}, calculated {:?}",
            signature,
            attr.function_id_bytes().unwrap_or([0; 4]),
            calculated_id
        ),
    )
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

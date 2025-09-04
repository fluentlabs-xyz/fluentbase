use crate::abi::{error::ABIError, function::FunctionABI, constructor::ConstructorABI};
use crate::method::CONSTRUCTOR_METHOD;
use convert_case::{Case, Casing};
use quote::ToTokens;
use std::ops::Deref;
use syn::{spanned::Spanned, FnArg, ReturnType, Signature};

/// Wrapper around `syn::Signature`
#[derive(Debug, Clone)]
pub struct ParsedSignature(Signature);

impl Deref for ParsedSignature {
    type Target = Signature;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ParsedSignature {
    /// Creates a new `ParsedSignature`
    pub fn new(signature: Signature) -> Self {
        Self(signature)
    }

    /// Returns the function name in Rust style (snake_case)
    pub fn rust_name(&self) -> String {
        self.0.ident.to_string()
    }

    /// Returns the function name in Solidity style (camelCase)
    pub fn sol_name(&self) -> String {
        self.rust_name().to_case(Case::Camel)
    }

    /// Returns the function's input arguments as a vector of (name, type) pairs
    pub fn input_args(&self) -> Vec<(String, String)> {
        self.0
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(pat_type) => {
                    let name = match &*pat_type.pat {
                        syn::Pat::Ident(ident) => ident.ident.to_string(),
                        _ => "_".to_string(),
                    };
                    let ty = pat_type.ty.to_token_stream().to_string();
                    Some((name, ty))
                }
                _ => None,
            })
            .collect()
    }

    /// Returns the function's return type
    pub fn output(&self) -> &ReturnType {
        &self.0.output
    }

    /// Returns the function parameters (input arguments)
    /// Used for codec generation and other parameter processing
    pub fn parameters(&self) -> Vec<&FnArg> {
        self.inputs_without_receiver()
    }

    /// Returns information about function return type
    /// Used for codec generation to determine if function has a return value
    pub fn return_type(&self) -> Vec<String> {
        match &self.0.output {
            ReturnType::Default => Vec::new(),
            ReturnType::Type(_, ty) => match &**ty {
                syn::Type::Tuple(tuple) => {
                    if tuple.elems.is_empty() {
                        Vec::new()
                    } else {
                        tuple
                            .elems
                            .iter()
                            .map(|ty| ty.to_token_stream().to_string())
                            .collect()
                    }
                }
                ty => vec![ty.to_token_stream().to_string()],
            },
        }
    }

    pub fn inputs(&self) -> &syn::punctuated::Punctuated<FnArg, syn::token::Comma> {
        &self.0.inputs
    }

    pub fn inputs_without_receiver(&self) -> Vec<&FnArg> {
        self.0
            .inputs
            .iter()
            .filter(|arg| !matches!(arg, FnArg::Receiver(_)))
            .collect()
    }

    /// Returns the ABI representation of the function
    pub fn function_abi(&self) -> Result<FunctionABI, ABIError> {
        FunctionABI::from_signature(&self.0)
    }
    pub fn constructor_abi(&self) -> Result<ConstructorABI, ABIError> {
        ConstructorABI::from_signature(&self.0)
    }

    pub fn is_fallback(&self) -> bool {
        self.0.ident == "fallback"
    }

    pub fn is_constructor(&self) -> bool {
        self.0.ident == CONSTRUCTOR_METHOD
    }


    /// Get span information for the signature for error reporting
    pub fn span(&self) -> proc_macro2::Span {
        self.0.span()
    }
}

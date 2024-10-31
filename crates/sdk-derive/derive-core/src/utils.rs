use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{Expr, Lit, Type, TypePath};

// Constants
const MAX_BYTES_SIZE: usize = 32;

// Error handling
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("Unsupported type: {0}")]
    UnsupportedType(String),
    #[error("Invalid array length")]
    InvalidArrayLength,
    #[error("Unsupported vector configuration")]
    UnsupportedVector,
    #[error("Invalid bytes size: {0}")]
    InvalidBytesSize(usize),
}

impl From<ConversionError> for syn::Error {
    fn from(err: ConversionError) -> Self {
        syn::Error::new(proc_macro2::Span::call_site(), err)
    }
}

// Type definitions
#[derive(Debug, Clone, PartialEq)]
pub enum SolidityType {
    Uint(u16),
    Int(u16),
    String,
    Bool,
    Address,
    Bytes,
    FixedBytes(u8),
    Array(Box<SolidityType>, Option<usize>),
    Tuple(Vec<SolidityType>),
}

// Trait for type conversion
pub trait IntoSolidityType {
    fn into_solidity_type(&self) -> Result<SolidityType, ConversionError>;
}

pub fn rust_type_to_sol(ty: &Type) -> Result<TokenStream, ConversionError> {
    match ty {
        Type::Array(ty) => convert_array_type(ty),
        Type::Paren(ty) => convert_paren_type(ty),
        Type::Slice(ty) => convert_slice_type(ty),
        Type::Tuple(ty) => convert_tuple_type(ty),
        Type::Path(type_path) => convert_path_type(type_path),
        Type::Reference(type_ref) => rust_type_to_sol(&type_ref.elem),
        _ => Err(ConversionError::UnsupportedType(
            ty.to_token_stream().to_string(),
        )),
    }
}

fn convert_array_type(ty: &syn::TypeArray) -> Result<TokenStream, ConversionError> {
    let len = match &ty.len {
        Expr::Lit(expr_lit) => match &expr_lit.lit {
            Lit::Int(lit_int) => lit_int
                .base10_parse::<u64>()
                .map_err(|_| ConversionError::InvalidArrayLength)?,
            _ => return Err(ConversionError::InvalidArrayLength),
        },
        _ => return Err(ConversionError::InvalidArrayLength),
    };

    let elem_type = rust_type_to_sol(&ty.elem)?;
    Ok(quote! { #elem_type[#len] })
}

fn convert_paren_type(ty: &syn::TypeParen) -> Result<TokenStream, ConversionError> {
    rust_type_to_sol(&ty.elem)
}

fn convert_slice_type(ty: &syn::TypeSlice) -> Result<TokenStream, ConversionError> {
    let elem_type = rust_type_to_sol(&ty.elem)?;
    Ok(quote! { #elem_type[] })
}

fn convert_tuple_type(ty: &syn::TypeTuple) -> Result<TokenStream, ConversionError> {
    let elems = ty
        .elems
        .iter()
        .map(rust_type_to_sol)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(quote! { (#(#elems),*) })
}

fn convert_path_type(type_path: &TypePath) -> Result<TokenStream, ConversionError> {
    let ident = &type_path
        .path
        .segments
        .last()
        .ok_or_else(|| ConversionError::UnsupportedType("Empty path".to_string()))?
        .ident;

    match ident.to_string().as_str() {
        "String" | "str" => Ok(quote! { string }),
        "bool" => Ok(quote! { bool }),
        s @ ("u8" | "u16" | "u32" | "u64" | "u128" | "u256" | "uint") => convert_uint_type(s),
        s @ ("i8" | "i16" | "i32" | "i64" | "i128" | "i256" | "int") => convert_int_type(s),
        "Address" => Ok(quote! { address }),
        "Bytes" => Ok(quote! { bytes }),
        "U256" => Ok(quote! { uint256 }),
        "Vec" => convert_vec_type(type_path),
        s if s.starts_with("bytes") => convert_bytes_type(s),
        _ => Ok(quote! { #ident }),
    }
}

fn convert_uint_type(s: &str) -> Result<TokenStream, ConversionError> {
    let size = match s {
        "u8" => "8",
        "u16" => "16",
        "u32" => "32",
        "u64" => "64",
        "u128" => "128",
        "u256" | "uint" => "256",
        _ => return Err(ConversionError::UnsupportedType(s.to_string())),
    };
    let ident = format_ident!("uint{}", size);
    Ok(quote! { #ident })
}

fn convert_int_type(s: &str) -> Result<TokenStream, ConversionError> {
    let size = match s {
        "i8" => "8",
        "i16" => "16",
        "i32" => "32",
        "i64" => "64",
        "i128" => "128",
        "i256" | "int" => "256",
        _ => return Err(ConversionError::UnsupportedType(s.to_string())),
    };
    let ident = format_ident!("int{}", size);
    Ok(quote! { #ident })
}

fn convert_vec_type(type_path: &TypePath) -> Result<TokenStream, ConversionError> {
    if let syn::PathArguments::AngleBracketed(args) = &type_path
        .path
        .segments
        .last()
        .ok_or_else(|| ConversionError::UnsupportedVector)?
        .arguments
    {
        if let Some(syn::GenericArgument::Type(arg_ty)) = args.args.first() {
            let elem_type = rust_type_to_sol(arg_ty)?;
            Ok(quote! { #elem_type[] })
        } else {
            Err(ConversionError::UnsupportedVector)
        }
    } else {
        Err(ConversionError::UnsupportedVector)
    }
}

fn convert_bytes_type(s: &str) -> Result<TokenStream, ConversionError> {
    if s == "bytes" {
        Ok(quote! { bytes })
    } else if s.len() > 5 {
        let size: usize = s[5..]
            .parse()
            .map_err(|_| ConversionError::InvalidBytesSize(0))?;
        if size > 0 && size <= MAX_BYTES_SIZE {
            let ident = format_ident!("bytes{}", size);
            Ok(quote! { #ident })
        } else {
            Err(ConversionError::InvalidBytesSize(size))
        }
    } else {
        Err(ConversionError::UnsupportedType(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_numeric_types() {
        use quote::format_ident;
        use syn::{parse_quote, TypePath};

        let test_cases = vec![
            ("u8", "uint8"),
            ("u16", "uint16"),
            ("u32", "uint32"),
            ("u64", "uint64"),
            ("u128", "uint128"),
            ("U256", "uint256"),
        ];

        for (input, expected) in test_cases {
            let ident = format_ident!("{}", input);
            let ty: TypePath = parse_quote!(#ident);
            assert_eq!(
                convert_path_type(&ty).unwrap().to_string(),
                expected.to_string()
            );
        }
    }

    #[test]
    fn test_array_types() {
        let test_cases = vec![
            (parse_quote!([u8; 32]), "uint8 [32u64]"),
            (parse_quote!([bool; 16]), "bool [16u64]"),
        ];

        for (input, expected) in test_cases {
            assert_eq!(
                convert_array_type(&input).unwrap().to_string(),
                expected.to_string()
            );
        }
    }

    #[test]
    fn test_error_handling() {
        let invalid_bytes: TypePath = parse_quote!(bytes33);
        assert!(matches!(
            convert_path_type(&invalid_bytes),
            Err(ConversionError::InvalidBytesSize(_))
        ));

        let custom_type: TypePath = parse_quote!(CustomStruct);
        assert!(matches!(convert_path_type(&custom_type), Ok(_)));
    }
}

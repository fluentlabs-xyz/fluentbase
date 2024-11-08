use crypto_hashes::{digest::Digest, sha3::Keccak256};
use darling::FromMeta;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Lit,
    LitStr,
    Result,
    Token,
};

/// Function identifier attribute for custom routing configuration.
#[derive(Debug, FromMeta, Clone)]
pub struct FunctionIDAttribute {
    /// Optional validation flag for function signatures
    #[darling(default)]
    pub validate: Option<bool>,
    /// Function identifier representation
    #[darling(skip)]
    pub function_id: Option<FunctionIDType>,
}

/// Represents different ways to specify a function identifier.
#[derive(Debug, Clone)]
pub enum FunctionIDType {
    /// Function signature (e.g., "transfer(address,uint256)")
    Signature(String),
    /// Hexadecimal representation (e.g., "0x12345678")
    HexString(String),
    /// Raw byte array
    ByteArray([u8; 4]),
}

pub const SELECTOR_SIZE: usize = 4;
const HEX_STRING_LENGTH: usize = 10; // "0x" + 8 hex chars

impl FunctionIDAttribute {
    /// Validates the Solidity function signature format.
    fn validate_signature(&self, signature: &str) -> Result<()> {
        if !signature.ends_with(')') || !signature.contains('(') {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "Invalid Solidity function signature: '{}'. \
                     Expected format: 'functionName(type1,type2,...)'",
                    signature
                ),
            ));
        }
        Ok(())
    }

    /// Returns the function ID as a hexadecimal string.
    pub fn function_id_hex(&self) -> Result<String> {
        self.function_id
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
            .and_then(|id| {
                Ok(match id {
                    FunctionIDType::Signature(sig) => {
                        format!("0x{}", hex::encode(&compute_keccak256(sig)?))
                    }
                    FunctionIDType::HexString(hex_str) => hex_str.clone(),
                    FunctionIDType::ByteArray(arr) => format!("0x{}", hex::encode(arr)),
                })
            })
    }

    /// Returns the function ID as a byte array.
    pub fn function_id_bytes(&self) -> Result<[u8; 4]> {
        self.function_id
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
            .and_then(|id| match id {
                FunctionIDType::Signature(sig) => compute_keccak256(sig),
                FunctionIDType::HexString(hex_str) => parse_hex_string(hex_str),
                FunctionIDType::ByteArray(arr) => Ok(*arr),
            })
    }

    /// Returns the original function signature if available.
    pub fn signature(&self) -> Option<String> {
        self.function_id.as_ref().and_then(|id| match id {
            FunctionIDType::Signature(sig) => Some(sig.clone()),
            _ => None,
        })
    }
}

/// Computes the Keccak256 hash of a signature and returns the first 4 bytes.
fn compute_keccak256(signature: &str) -> Result<[u8; 4]> {
    let mut hasher = Keccak256::new();
    hasher.update(signature.as_bytes());
    let mut selector = [0u8; SELECTOR_SIZE];
    selector.copy_from_slice(&hasher.finalize()[..SELECTOR_SIZE]);
    Ok(selector)
}

/// Parses a hexadecimal string into a 4-byte array.
fn parse_hex_string(hex_str: &str) -> Result<[u8; 4]> {
    let bytes = hex::decode(hex_str.trim_start_matches("0x"))
        .map_err(|e| syn::Error::new(Span::call_site(), format!("Invalid hex string: {}", e)))?;

    if bytes.len() != SELECTOR_SIZE {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "Invalid hex string length. Expected {} bytes, found {}",
                SELECTOR_SIZE,
                bytes.len()
            ),
        ));
    }

    let mut selector = [0u8; SELECTOR_SIZE];
    selector.copy_from_slice(&bytes);
    Ok(selector)
}

impl Parse for FunctionIDAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        let function_id = if input.lookahead1().peek(LitStr) {
            parse_string_literal(input)?
        } else if input.lookahead1().peek(syn::token::Bracket) {
            parse_byte_array(input)?
        } else {
            return Err(syn::Error::new(
                input.span(),
                "Expected either a string literal or a byte array [u8; 4]",
            ));
        };

        let validate = if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            parse_validate_attribute(input)?
        } else {
            None
        };

        Ok(FunctionIDAttribute {
            validate,
            function_id: Some(function_id),
        })
    }
}

/// Parses a string literal into a FunctionIDType.
fn parse_string_literal(input: ParseStream) -> Result<FunctionIDType> {
    let lit_str: LitStr = input.parse()?;
    let value = lit_str.value();

    if value.starts_with("0x") && value.len() == HEX_STRING_LENGTH {
        Ok(FunctionIDType::HexString(value))
    } else if value.contains('(') && value.ends_with(')') {
        Ok(FunctionIDType::Signature(value))
    } else {
        Err(syn::Error::new(
            lit_str.span(),
            "Invalid function ID format. Expected either a Solidity function signature \
             (e.g., 'transfer(address,uint256)') or a 4-byte hex string (e.g., '0x12345678')",
        ))
    }
}

/// Parses a byte array into a FunctionIDType.
fn parse_byte_array(input: ParseStream) -> Result<FunctionIDType> {
    let content;
    syn::bracketed!(content in input);

    let bytes: Vec<u8> = content
        .parse_terminated(Lit::parse, Token![,])?
        .into_iter()
        .map(|lit| match lit {
            syn::Lit::Int(lit_int) => lit_int.base10_parse::<u8>().map_err(|_| {
                syn::Error::new_spanned(&lit_int, "Invalid byte value. Expected 0-255")
            }),
            _ => Err(syn::Error::new_spanned(&lit, "Expected u8 literal")),
        })
        .collect::<Result<_>>()?;

    if bytes.len() != SELECTOR_SIZE {
        return Err(syn::Error::new(
            content.span(),
            format!(
                "Expected exactly {} bytes, found {}",
                SELECTOR_SIZE,
                bytes.len()
            ),
        ));
    }

    Ok(FunctionIDType::ByteArray(bytes.try_into().unwrap()))
}

/// Parses the validate attribute.
fn parse_validate_attribute(input: ParseStream) -> Result<Option<bool>> {
    let meta = input.parse::<syn::Meta>()?;

    match meta {
        syn::Meta::List(list) if list.path.is_ident("validate") => {
            let nested = list
                .parse_args_with(Punctuated::<syn::Expr, Token![,]>::parse_terminated)
                .map_err(|e| {
                    syn::Error::new(
                        list.span(),
                        format!(
                            "Invalid 'validate' attribute: {}. Expected 'validate(true/false)'",
                            e
                        ),
                    )
                })?;

            if nested.len() != 1 {
                return Err(syn::Error::new(
                    list.span(),
                    format!(
                        "Expected one argument for 'validate', found {}",
                        nested.len()
                    ),
                ));
            }

            match &nested[0] {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Bool(lit_bool),
                    ..
                }) => Ok(Some(lit_bool.value)),
                _ => Err(syn::Error::new(
                    nested[0].span(),
                    "Expected a boolean literal for 'validate'",
                )),
            }
        }
        _ => Err(syn::Error::new(
            meta.span(),
            "Expected 'validate(true)' or 'validate(false)'",
        )),
    }
}

impl ToTokens for FunctionIDAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let validate = self.validate.unwrap_or(true);

        match &self.function_id {
            Some(function_id) => {
                let function_id_hex = match self.function_id_hex() {
                    Ok(hex) => hex,
                    Err(e) => {
                        tokens.extend(create_error_tokens(&e));
                        return;
                    }
                };

                let function_id_bytes = match self.function_id_bytes() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tokens.extend(create_error_tokens(&e));
                        return;
                    }
                };

                if let FunctionIDType::Signature(sig) = function_id {
                    if validate {
                        if let Err(e) = self.validate_signature(sig) {
                            tokens.extend(create_error_tokens(&e));
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
            }
            None => {
                tokens.extend(quote! {
                    compile_error!("FunctionID attribute requires a value");
                });
            }
        }
    }
}

/// Creates token stream for compile errors.
fn create_error_tokens(error: &syn::Error) -> TokenStream2 {
    let error_msg = error.to_string();
    quote! { compile_error!(#error_msg); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_parse_signature() {
        // Valid signature
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };

        assert!(matches!(
            &attr.function_id,
            Some(FunctionIDType::Signature(sig)) if sig == "transfer(address,uint256)"
        ));

        // Check hex representation
        let hex = attr.function_id_hex().unwrap();
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 10); // "0x" + 8 chars

        // Check bytes
        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn test_parse_hex_string() {
        // Valid hex string
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"
        };

        assert!(matches!(
            &attr.function_id,
            Some(FunctionIDType::HexString(hex)) if hex == "0x12345678"
        ));

        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
    }
    #[test]
    fn test_parse_byte_array() {
        // Valid byte array
        let attr: FunctionIDAttribute = parse_quote! {
            [1, 2, 3, 4]
        };

        assert!(matches!(
            attr.function_id,
            Some(FunctionIDType::ByteArray(bytes)) if bytes == [1, 2, 3, 4]
        ));

        let hex = attr.function_id_hex().unwrap();
        assert_eq!(hex, "0x01020304");
    }

    #[test]
    fn test_validate_attribute() {
        // With validate(true)
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(true)
        };
        assert_eq!(attr.validate, Some(true));

        // With validate(false)
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(false)
        };
        assert_eq!(attr.validate, Some(false));
    }

    #[test]
    fn test_signature_validation() {
        let attr = FunctionIDAttribute {
            validate: Some(true),
            function_id: Some(FunctionIDType::Signature(
                "transfer(address,uint256)".to_string(),
            )),
        };
        assert!(attr.validate_signature("transfer(address,uint256)").is_ok());

        let attr = FunctionIDAttribute {
            validate: Some(true),
            function_id: Some(FunctionIDType::Signature("invalid_signature".to_string())),
        };
        assert!(attr.validate_signature("invalid_signature").is_err());
    }
    #[test]
    #[should_panic(expected = "Invalid function ID format")]
    fn test_invalid_hex_string() {
        let _attr: FunctionIDAttribute = parse_quote! {
            "0x123"  // Invalid format - too short
        };
    }

    #[test]
    fn test_valid_hex_string() {
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"  // Correct length
        };
        assert!(attr.function_id_bytes().is_ok());
    }

    #[test]
    fn test_invalid_hex_content() {
        let attr: FunctionIDAttribute = parse_quote! {
            "0x1234567z"  // Invalid hex character
        };
        assert!(attr.function_id_bytes().is_err());
    }

    #[test]
    #[should_panic(expected = "Expected exactly 4 bytes")]
    fn test_invalid_byte_array() {
        let attr: FunctionIDAttribute = parse_quote! {
            [1, 2, 3]  // Only 3 bytes
        };
        attr.function_id_bytes().unwrap();
    }

    #[test]
    fn test_to_tokens() {
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };
        let tokens = quote! { #attr };
        let tokens_str = tokens.to_string();

        assert!(tokens_str.contains("FUNCTION_SIGNATURE"));
        assert!(tokens_str.contains("FUNCTION_ID_HEX"));
        assert!(tokens_str.contains("FUNCTION_ID_BYTES"));
    }
}

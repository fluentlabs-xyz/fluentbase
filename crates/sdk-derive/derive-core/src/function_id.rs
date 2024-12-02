use crypto_hashes::{digest::Digest, sha3::Keccak256};
use darling::FromMeta;
use proc_macro2::{Span, TokenStream as TokenStream2};
use proc_macro_error::emit_error;
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

#[derive(Debug, FromMeta, Clone)]
pub struct FunctionIDAttribute {
    /// Optional validation flag for function signatures
    #[darling(default)]
    pub validate: Option<bool>,
    /// Function identifier representation
    #[darling(skip)]
    pub function_id: Option<FunctionIDValue>,
}

#[derive(Debug, Clone)]
pub struct FunctionIDValue {
    span: Span,
    value: FunctionIDType,
}

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
    pub fn span(&self) -> Option<Span> {
        self.function_id.as_ref().map(|id| id.span)
    }

    /// Validates the Solidity function signature format.
    fn validate_signature(&self, signature: &str, span: Span) -> Result<()> {
        if !signature.ends_with(')') || !signature.contains('(') {
            return Err(syn::Error::new(
                span,
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
                Ok(match &id.value {
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
            .and_then(|id| match &id.value {
                FunctionIDType::Signature(sig) => {
                    compute_keccak256(sig).map_err(|e| syn::Error::new(id.span, e.to_string()))
                }
                FunctionIDType::HexString(hex_str) => {
                    parse_hex_string(hex_str).map_err(|e| syn::Error::new(id.span, e.to_string()))
                }
                FunctionIDType::ByteArray(arr) => Ok(*arr),
            })
    }

    /// Returns the original function signature if available.
    pub fn signature(&self) -> Option<String> {
        self.function_id.as_ref().and_then(|id| match &id.value {
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
            let lit_str: LitStr = input.parse()?;
            let span = lit_str.span();
            let value = lit_str.value();

            let value = if value.starts_with("0x") && value.len() == HEX_STRING_LENGTH {
                FunctionIDType::HexString(value)
            } else if value.contains('(') && value.ends_with(')') {
                FunctionIDType::Signature(value)
            } else {
                return Err(syn::Error::new(
                    span,
                    "Invalid function ID format. Expected either a Solidity function signature \
                     (e.g., 'transfer(address,uint256)') or a 4-byte hex string (e.g., '0x12345678')"
                ));
            };

            FunctionIDValue { span, value }
        } else if input.lookahead1().peek(syn::token::Bracket) {
            let content;
            let bracket_token = syn::bracketed!(content in input);
            let span = bracket_token.span.span();

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
                    span,
                    format!(
                        "Expected exactly {} bytes, found {}",
                        SELECTOR_SIZE,
                        bytes.len()
                    ),
                ));
            }

            FunctionIDValue {
                span,
                value: FunctionIDType::ByteArray(bytes.try_into().unwrap()),
            }
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

        let (function_id, span) = match &self.function_id {
            Some(id) => (&id.value, id.span),
            None => {
                emit_error!(Span::call_site(), "FunctionID attribute requires a value");
                return;
            }
        };

        let function_id_bytes = match self.function_id_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                emit_error!(span, "{}", err);
                return;
            }
        };

        let function_id_hex = format!("0x{}", hex::encode(function_id_bytes));

        if let FunctionIDType::Signature(sig) = function_id {
            if validate {
                if let Err(err) = self.validate_signature(sig, span) {
                    emit_error!(span, "{}", err);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_parse_signature() {
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };

        assert!(matches!(
            &attr.function_id,
            Some(FunctionIDValue { value: FunctionIDType::Signature(sig), .. })
            if sig == "transfer(address,uint256)"
        ));

        let hex = attr.function_id_hex().unwrap();
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 10);

        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn test_parse_hex_string() {
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"
        };

        assert!(matches!(
            &attr.function_id,
            Some(FunctionIDValue { value: FunctionIDType::HexString(hex), .. })
            if hex == "0x12345678"
        ));

        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_parse_byte_array() {
        let attr: FunctionIDAttribute = parse_quote! {
            [1, 2, 3, 4]
        };

        match &attr.function_id {
            Some(FunctionIDValue {
                value: FunctionIDType::ByteArray(bytes),
                ..
            }) => {
                assert_eq!(bytes, &[1, 2, 3, 4]);
            }
            other => panic!("Expected ByteArray, got {:?}", other),
        }

        let hex = attr.function_id_hex().unwrap();
        assert_eq!(hex, "0x01020304");
    }

    #[test]
    fn test_validate_attribute() {
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(true)
        };
        assert_eq!(attr.validate, Some(true));

        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(false)
        };
        assert_eq!(attr.validate, Some(false));
    }

    #[test]
    fn test_signature_validation() {
        let attr = FunctionIDAttribute {
            validate: Some(true),
            function_id: Some(FunctionIDValue {
                span: Span::call_site(),
                value: FunctionIDType::Signature("transfer(address,uint256)".to_string()),
            }),
        };
        assert!(attr
            .validate_signature("transfer(address,uint256)", Span::call_site())
            .is_ok());

        let attr = FunctionIDAttribute {
            validate: Some(true),
            function_id: Some(FunctionIDValue {
                span: Span::call_site(),
                value: FunctionIDType::Signature("invalid_signature".to_string()),
            }),
        };
        assert!(attr
            .validate_signature("invalid_signature", Span::call_site())
            .is_err());
    }

    #[test]
    #[should_panic(expected = "Invalid function ID format")]
    fn test_invalid_hex_string() {
        let _attr: FunctionIDAttribute = parse_quote! {
            "0x123"
        };
    }

    #[test]
    fn test_valid_hex_string() {
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"
        };
        assert!(attr.function_id_bytes().is_ok());
    }

    #[test]
    fn test_invalid_hex_content() {
        let attr: FunctionIDAttribute = parse_quote! {
            "0x1234567z"
        };
        assert!(attr.function_id_bytes().is_err());
    }

    #[test]
    #[should_panic(expected = "Expected exactly 4 bytes")]
    fn test_invalid_byte_array() {
        let attr: FunctionIDAttribute = parse_quote! {
            [1, 2, 3]
        };
        attr.function_id_bytes().unwrap();
    }
}

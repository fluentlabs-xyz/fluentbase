use darling::FromMeta;
use hex;
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    LitStr,
    Result,
    Token,
};

#[derive(Debug, FromMeta)]
pub struct FunctionIDAttribute {
    #[darling(default)]
    validate: Option<bool>,
    #[darling(skip)]
    function_id: Option<FunctionIDType>,
}

#[derive(Debug)]
enum FunctionIDType {
    Signature(String),
    HexString(String),
    ByteArray([u8; 4]),
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

impl FunctionIDAttribute {
    fn validate_signature(&self, signature: &str) -> Result<()> {
        if !signature.ends_with(")") || !signature.contains("(") {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("Invalid Solidity function signature: '{}'. Expected format: 'functionName(type1,type2,...)'", signature)
            ));
        }
        // TODO: Add more detailed signature validation here
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

impl ToTokens for FunctionIDAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream) {
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

fn error_tokens(e: syn::Error) -> TokenStream {
    let error_msg = e.to_string();
    quote! { compile_error!(#error_msg); }
}

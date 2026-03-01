use crate::utils::selector::{calculate_keccak256, parse_hex_string};
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

/// Basic type for a function selector - just 4 bytes
pub type FunctionID = [u8; 4];

/// Function identifier attribute for custom routing configuration.
///
/// # Example
/// ```rust, ignore
/// #[function_id("transfer(address,uint256)")]
/// fn transfer(&self, to: Address, amount: U256) -> bool { /* ... */ }
///
/// // With validation enabled
/// #[function_id("0xabcdef12", validate(true))]
/// fn another_method(&self, value: u32) -> u32 { /* ... */ }
/// ```
#[derive(Debug, FromMeta, Clone)]
pub struct FunctionIDAttribute {
    /// Optional validation flag for function signatures
    ///
    /// When set to `true`, validates that the provided function ID
    /// matches the one derived from the method signature.
    /// Defaults to `false`, allowing custom function IDs without validation.
    #[darling(default = "default_validate")]
    pub validate: Option<bool>,

    /// Function identifier as 4 bytes
    #[darling(skip)]
    pub selector: Option<FunctionID>,

    /// Original input for error reporting
    #[darling(skip)]
    pub original_input: Option<Input>,
}

/// Represents the original input format for error reporting and generation
#[derive(Debug, Clone)]
pub enum Input {
    /// Function signature (e.g., "transfer(address,uint256)")
    Signature(String),
    /// Hexadecimal representation (e.g., "0x12345678")
    HexString(String),
    /// Raw byte array notation
    ByteArray,
}

/// Default value for the validate flag (false - no validation by default)
fn default_validate() -> Option<bool> {
    Some(false)
}

pub const SELECTOR_SIZE: usize = 4;
const HEX_STRING_LENGTH: usize = 10; // "0x" + 8 hex chars

impl FunctionIDAttribute {
    /// Creates a new FunctionIDAttribute from a signature
    pub fn from_signature(signature: &str) -> Result<Self> {
        if !Self::is_valid_signature(signature) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "Invalid Solidity function signature: '{signature}'. \
                 Expected format: 'functionName(type1,type2,...)'"
                ),
            ));
        }

        let sanitized = sanitize_signature(signature).ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                format!("Failed to sanitize signature: '{signature}'"),
            )
        })?;

        Ok(Self {
            validate: default_validate(),
            selector: Some(calculate_keccak256(&sanitized)),
            original_input: Some(Input::Signature(signature.to_string())),
        })
    }

    /// Creates a new FunctionIDAttribute from a hex string
    pub fn from_hex(hex_str: &str) -> Result<Self> {
        let selector = parse_hex_string(hex_str)?;

        Ok(Self {
            validate: default_validate(),
            selector: Some(selector),
            original_input: Some(Input::HexString(hex_str.to_string())),
        })
    }

    /// Creates a new FunctionIDAttribute from a byte array
    pub fn from_bytes(bytes: FunctionID) -> Self {
        Self {
            validate: default_validate(),
            selector: Some(bytes),
            original_input: Some(Input::ByteArray),
        }
    }

    /// Validates if a string is a valid Solidity function signature
    fn is_valid_signature(signature: &str) -> bool {
        signature.ends_with(')') && signature.contains('(')
    }

    /// Returns the function ID as a hexadecimal string.
    ///
    /// # Returns
    /// * `Ok(String)` - The hexadecimal representation (e.g. "0x12345678")
    /// * `Err(Error)` - If the function ID is not set
    pub fn function_id_hex(&self) -> Result<String> {
        self.selector
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
            .map(|bytes| format!("0x{}", hex::encode(bytes)))
    }

    /// Returns the function ID as a byte array.
    ///
    /// # Returns
    /// * `Ok([u8; 4])` - The 4-byte function selector
    /// * `Err(Error)` - If the function ID is not set
    pub fn function_id_bytes(&self) -> Result<FunctionID> {
        self.selector
            .ok_or_else(|| syn::Error::new(Span::call_site(), "Function ID is not set"))
    }

    /// Returns the original function signature if available.
    ///
    /// # Returns
    /// * `Some(String)` - The function signature if specified
    /// * `None` - If the function ID was specified as a hex string or byte array
    #[must_use]
    pub fn signature(&self) -> Option<String> {
        if let Some(Input::Signature(sig)) = &self.original_input {
            Some(sig.clone())
        } else {
            None
        }
    }

    /// Checks if validation is enabled for this attribute.
    ///
    /// # Returns
    /// * `true` - If validation is explicitly enabled
    /// * `false` - If validation is explicitly disabled or not specified
    #[must_use]
    pub fn is_validation_enabled(&self) -> bool {
        self.validate.unwrap_or(false)
    }
}

impl Parse for FunctionIDAttribute {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.lookahead1().peek(LitStr) {
            let lit_str: LitStr = input.parse()?;
            let value = lit_str.value();

            let attr = if value.starts_with("0x") && value.len() == HEX_STRING_LENGTH {
                Self::from_hex(&value)?
            } else if Self::is_valid_signature(&value) {
                Self::from_signature(&value)?
            } else {
                return Err(syn::Error::new(
                    lit_str.span(),
                    "Invalid function ID format. Expected either a Solidity function signature \
                     (e.g., 'transfer(address,uint256)') or a 4-byte hex string (e.g., '0x12345678')",
                ));
            };

            // Parse the validation attribute if present
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
                let validate = parse_validate_attribute(input)?;
                return Ok(Self { validate, ..attr });
            }

            Ok(attr)
        } else if input.lookahead1().peek(syn::token::Bracket) {
            let content;
            let bracket_token = syn::bracketed!(content in input);
            let span = bracket_token.span.span();

            // Parse the byte array directly
            let bytes: Vec<u8> = content
                .parse_terminated(Lit::parse, Token![,])?
                .into_iter()
                .map(|lit| parse_u8_literal(&lit))
                .collect::<Result<Vec<u8>>>()?;

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

            let mut attr = Self::from_bytes(bytes.try_into().unwrap());

            // Parse the validation attribute if present
            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
                let validate = parse_validate_attribute(input)?;
                attr.validate = validate;
            }

            Ok(attr)
        } else {
            Err(syn::Error::new(
                input.span(),
                "Expected either a string literal or a byte array [u8; 4]",
            ))
        }
    }
}

/// Parses a u8 literal from a Lit.
///
/// # Arguments
/// * `lit` - The literal to parse
///
/// # Returns
/// * `Result<u8>` - The parsed u8 value or an error
fn parse_u8_literal(lit: &Lit) -> Result<u8> {
    match lit {
        syn::Lit::Int(lit_int) => lit_int
            .base10_parse::<u8>()
            .map_err(|_| syn::Error::new_spanned(lit_int, "Invalid byte value. Expected 0-255")),
        _ => Err(syn::Error::new_spanned(lit, "Expected u8 literal")),
    }
}

/// Parses the validate attribute.
///
/// # Arguments
/// * `input` - The parse stream to read from
///
/// # Returns
/// * `Result<Option<bool>>` - The parsed validate value or an error
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
                            "Invalid 'validate' attribute: {e}. Expected 'validate(true/false)'"
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
        // Get function ID or emit error if not set
        let function_id_bytes = match self.function_id_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                emit_error!(Span::call_site(), "{}", err);
                return;
            }
        };

        // Format as hex string
        let function_id_hex = format!("0x{}", hex::encode(function_id_bytes));

        // Include signature for signature-based function IDs
        if let Some(signature) = self.signature() {
            tokens.extend(quote! {
                const FUNCTION_SIGNATURE: &str = #signature;
            });
        }

        // Include hex and bytes constants
        tokens.extend(quote! {
            const FUNCTION_ID_HEX: &str = #function_id_hex;
            const FUNCTION_ID_BYTES: [u8; 4] = [#(#function_id_bytes),*];
        });
    }
}

/// Splits the parameter list into arguments,
/// handling nested tuples/arrays so commas inside brackets are ignored.
fn split_args_types(params: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut curr = String::new();
    let mut depth = 0;

    for c in params.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                curr.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                curr.push(c);
            }
            ',' if depth == 0 => {
                // Only split at top-level commas
                args.push(curr.trim().to_string());
                curr.clear();
            }
            _ => curr.push(c),
        }
    }
    if !curr.trim().is_empty() {
        args.push(curr.trim().to_string());
    }
    args
}

/// Removes argument names, storage modifiers, and keeps only the type.
/// For example:
///   "uint256 amount"              -> "uint256"
///   "(address,bool) memory data"  -> "(address,bool)"
///   "uint256[3] calldata arr"     -> "uint256[3]"
fn extract_type(arg: &str) -> String {
    let mut part = arg.trim();

    // Remove storage/location modifiers if present
    for modifier in ["memory", "calldata", "storage"] {
        if let Some(idx) = part.find(&format!(" {}", modifier)) {
            part = &part[..idx];
        }
    }

    // Remove trailing argument names.
    // Find the last whitespace outside any nested tuple/array to separate type from name.
    let mut idx = part.len();
    let mut paren = 0;
    for (i, c) in part.chars().rev().enumerate() {
        match c {
            ')' | ']' => paren += 1,
            '(' | '[' => paren -= 1,
            _ => {}
        }
        if c.is_whitespace() && paren == 0 {
            idx = part.len() - i - 1;
            break;
        }
    }
    part[..idx].trim_end().to_string()
}

/// Converts a Solidity function declaration to a canonical signature string,
/// e.g. `function foo(uint256[3] memory arr, (address,bool) data)`
///   -> "foo(uint256[3],(address,bool))"
fn sanitize_signature(decl: &str) -> Option<String> {
    let start = decl.find('(')?;
    let end = decl.rfind(')')?;
    let name = decl[..start].split_whitespace().last()?;
    let params = &decl[start + 1..end];
    let types: Vec<_> = split_args_types(params)
        .iter()
        .map(|s| extract_type(s))
        .collect();
    let sig = format!("{}({})", name, types.join(","));
    Some(sig.replace(" ", ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use quote::{quote, ToTokens};
    use syn::parse_quote;

    #[test]
    fn test_parse_signature() {
        // Test valid signature
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };

        // Verify original input is stored correctly
        assert!(matches!(
            &attr.original_input,
            Some(Input::Signature(sig)) if sig == "transfer(address,uint256)"
        ));

        // Check if hexadecimal representation is correct
        let hex = attr.function_id_hex().unwrap();
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 10); // "0x" + 8 characters

        // Check if bytes are valid
        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes.len(), 4);
    }

    #[test]
    fn test_parse_hex_string() {
        // Test valid hexadecimal string
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"
        };

        // Verify original input is stored correctly
        assert!(matches!(
            &attr.original_input,
            Some(Input::HexString(hex)) if hex == "0x12345678"
        ));

        // Check if bytes are valid
        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_parse_byte_array() {
        // Test valid byte array
        let attr: FunctionIDAttribute = parse_quote! {
            [1, 2, 3, 4]
        };

        // Verify original input is stored correctly
        assert!(matches!(&attr.original_input, Some(Input::ByteArray)));

        // Check if bytes are valid
        let bytes = attr.function_id_bytes().unwrap();
        assert_eq!(bytes, [1, 2, 3, 4]);

        // Check if hexadecimal representation is correct
        let hex = attr.function_id_hex().unwrap();
        assert_eq!(hex, "0x01020304");
    }

    #[test]
    fn test_validation_settings() {
        // Test default validation (false)
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };
        assert!(!attr.is_validation_enabled());

        // Test explicit validation enabled
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(true)
        };
        assert!(attr.is_validation_enabled());

        // Test explicit validation disabled
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)", validate(false)
        };
        assert!(!attr.is_validation_enabled());
    }

    #[test]
    fn test_invalid_inputs() {
        // Test invalid function signature (missing parentheses)
        let result = syn::parse2::<FunctionIDAttribute>(quote! { "transfer_address_uint256" });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid function ID format"));

        // Test too short hexadecimal string
        let result = syn::parse2::<FunctionIDAttribute>(quote! { "0x123" });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid function ID format"));

        // Test invalid characters in hexadecimal string
        let result = syn::parse2::<FunctionIDAttribute>(quote! { "0x1234567z" });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid hex string"));

        // Test incomplete byte array
        let result = syn::parse2::<FunctionIDAttribute>(quote! { [1, 2, 3] });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected exactly 4 bytes"));
    }

    #[test]
    fn test_token_generation() {
        // Test token generation from function signature
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };
        let mut tokens = TokenStream::new();
        attr.to_tokens(&mut tokens);
        let tokens_str = tokens.to_string();

        assert!(tokens_str.contains("FUNCTION_SIGNATURE"));
        assert!(tokens_str.contains("FUNCTION_ID_HEX"));
        assert!(tokens_str.contains("FUNCTION_ID_BYTES"));

        // Test token generation from hexadecimal string (no FUNCTION_SIGNATURE)
        let attr: FunctionIDAttribute = parse_quote! {
            "0x12345678"
        };
        let mut tokens = TokenStream::new();
        attr.to_tokens(&mut tokens);
        let tokens_str = tokens.to_string();

        assert!(!tokens_str.contains("FUNCTION_SIGNATURE"));
        assert!(tokens_str.contains("FUNCTION_ID_HEX"));
        assert!(tokens_str.contains("FUNCTION_ID_BYTES"));
    }

    #[test]
    fn test_format_conversions() {
        // Test conversions between different formats
        let attr: FunctionIDAttribute = parse_quote! {
            "transfer(address,uint256)"
        };

        // Get signature, bytes and hex representation
        assert!(attr.signature().is_some());
        assert_eq!(attr.signature().unwrap(), "transfer(address,uint256)");

        let bytes = attr.function_id_bytes().unwrap();
        let hex = attr.function_id_hex().unwrap();

        // Create from hex and verify equivalence
        let attr2 = FunctionIDAttribute::from_hex(&hex).unwrap();
        assert_eq!(attr2.function_id_bytes().unwrap(), bytes);

        // Create from bytes and verify equivalence
        let attr3 = FunctionIDAttribute::from_bytes(bytes);
        assert_eq!(attr3.function_id_bytes().unwrap(), bytes);
        assert_eq!(attr3.function_id_hex().unwrap(), hex);
    }

    #[test]
    fn sanitize() {
        assert_eq!(
            sanitize_signature("function transfer(address to, uint256 amount)"),
            Some("transfer(address,uint256)".to_string())
        );
        assert_eq!(
            sanitize_signature("function mint(address,uint256)"),
            Some("mint(address,uint256)".to_string())
        );
        assert_eq!(
            sanitize_signature("function foo(uint8[] memory values, string calldata message)"),
            Some("foo(uint8[],string)".to_string())
        );
        assert_eq!(
            sanitize_signature("setData(uint256[3] memory arr, (address,bool) data)"),
            Some("setData(uint256[3],(address,bool))".to_string())
        );
        assert_eq!(sanitize_signature("empty()"), Some("empty()".to_string()));
        assert_eq!(
            sanitize_signature("function complex((uint256,(bool,address[]))  data , string s)"),
            Some("complex((uint256,(bool,address[])),string)".to_string())
        );
        assert_eq!(
            sanitize_signature("function nested(tuple(uint256,bool)[] memory arr)"),
            Some("nested(tuple(uint256,bool)[])".to_string())
        );
        assert_eq!(
            sanitize_signature(
                "deep(uint256,(address, (bool, uint256[3]), (string, (bytes, (uint8, address[2]))))[] memory,(uint256, (bytes32, (address[], (bool, (uint256, bytes32[4]))))) calldata)"
            ),
            Some("deep(uint256,(address,(bool,uint256[3]),(string,(bytes,(uint8,address[2]))))[],(uint256,(bytes32,(address[],(bool,(uint256,bytes32[4]))))))".to_string())
        );
    }
}

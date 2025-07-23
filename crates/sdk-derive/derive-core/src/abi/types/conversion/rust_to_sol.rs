use crate::abi::types::SolType;
use syn::{self, GenericArgument, PathArguments, Type};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ConversionError {
    #[error("Unsupported type: {0}")]
    UnsupportedType(String),
    #[error("Invalid array length: {0}")]
    InvalidArrayLength(String),
    #[error("Invalid fixed bytes size: {0}")]
    InvalidBytesSize(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

impl From<ConversionError> for syn::Error {
    fn from(err: ConversionError) -> Self {
        syn::Error::new(proc_macro2::Span::call_site(), err.to_string())
    }
}

/// Convert Rust type to Solidity type
pub fn rust_to_sol(ty: &Type) -> Result<SolType, ConversionError> {
    match ty {
        Type::Path(type_path) => convert_path_type(type_path),
        Type::Reference(type_ref) => rust_to_sol(&type_ref.elem),
        Type::Array(array) => convert_array_type(array),
        Type::Tuple(tuple) => convert_tuple_type(tuple),
        Type::Slice(slice) => convert_slice_type(slice),
        _ => Err(ConversionError::UnsupportedType(format!(
            "Unsupported type: {ty:?}"
        ))),
    }
}

fn get_full_path(type_path: &syn::TypePath) -> Result<String, ConversionError> {
    let mut path = String::new();
    for segment in &type_path.path.segments {
        if !path.is_empty() {
            path.push_str("::");
        }
        path.push_str(&segment.ident.to_string());

        // Handle generic parameters
        if let PathArguments::AngleBracketed(args) = &segment.arguments {
            if let Some(GenericArgument::Type(Type::Path(inner_path))) = args.args.first() {
                path.push('<');
                path.push_str(&get_full_path(inner_path)?);
                path.push('>');
            }
        }
    }
    Ok(path)
}

fn convert_path_type(type_path: &syn::TypePath) -> Result<SolType, ConversionError> {
    let last_segment = type_path
        .path
        .segments
        .last()
        .ok_or_else(|| ConversionError::ParseError("Empty type path".into()))?;

    let type_name = last_segment.ident.to_string();

    // Try primitive types first
    if let Some(result) = convert_primitive_type(&type_name) {
        return Ok(result);
    }

    // Handle special types
    match type_name.as_str() {
        "Vec" => convert_vec_type(last_segment),
        "FixedBytes" => convert_fixed_bytes(&type_name, &last_segment.arguments),
        _ => {
            // Special handling for array types is done in convert_array_type
            // Check for unsupported generic parameters in other types
            if !matches!(last_segment.arguments, PathArguments::None) {
                return Err(ConversionError::UnsupportedType(format!(
                    "Generic parameters are not supported for type: {type_name}"
                )));
            }

            // Get full path for better type identification
            let full_path = get_full_path(type_path)?;

            // Treat any unknown type as a potential struct
            Ok(SolType::Struct {
                name: full_path,
                fields: Vec::new(),
            })
        }
    }
}
fn convert_primitive_type(type_name: &str) -> Option<SolType> {
    // Normalize case for matching
    let lower = type_name.to_ascii_lowercase();

    // Unsigned types
    if lower.starts_with('u') {
        if let Ok(bits) = lower[1..].parse::<usize>() {
            // Allowed sizes: 8, 16, ..., 256, 512, and all multiples of 8 up to 256, plus 24, 40, 48, ...
            if [
                8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152,
                160, 168, 176, 184, 192, 200, 208, 216, 224, 232, 240, 248, 256, 512,
            ]
            .contains(&bits)
            {
                return Some(SolType::Uint(bits));
            }
        }
    }

    // Signed types
    if lower.starts_with('i') {
        if let Ok(bits) = lower[1..].parse::<usize>() {
            if [
                8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152,
                160, 168, 176, 184, 192, 200, 208, 216, 224, 232, 240, 248, 256, 512,
            ]
            .contains(&bits)
            {
                return Some(SolType::Int(bits));
            }
        }
    }

    // Other primitive types
    match type_name {
        "bool" => Some(SolType::Bool),
        "Address" => Some(SolType::Address),
        "String" | "str" => Some(SolType::String),
        "Bytes" => Some(SolType::Bytes),
        _ => None,
    }
}

fn convert_vec_type(segment: &syn::PathSegment) -> Result<SolType, ConversionError> {
    if let PathArguments::AngleBracketed(args) = &segment.arguments {
        if let Some(GenericArgument::Type(elem_type)) = args.args.first() {
            let inner_type = rust_to_sol(elem_type)?;
            return Ok(SolType::Array(Box::new(inner_type)));
        }
    }
    Err(ConversionError::ParseError("Invalid Vec type".into()))
}

fn convert_array_type(array: &syn::TypeArray) -> Result<SolType, ConversionError> {
    let len = match &array.len {
        syn::Expr::Lit(expr_lit) => {
            if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                lit_int.base10_parse::<usize>().map_err(|_| {
                    ConversionError::InvalidArrayLength("Invalid array length".into())
                })?
            } else {
                return Err(ConversionError::InvalidArrayLength(
                    "Non-integer array length".into(),
                ));
            }
        }
        _ => {
            return Err(ConversionError::InvalidArrayLength(
                "Non-literal array length".into(),
            ))
        }
    };

    if len == 0 {
        return Err(ConversionError::InvalidArrayLength(
            "Zero-length arrays not supported".into(),
        ));
    }

    let elem_type = rust_to_sol(&array.elem)?;
    if let SolType::Uint(8) = elem_type {
        if len <= 32 {
            return Ok(SolType::FixedBytes(len));
        }
    }

    Ok(SolType::FixedArray(Box::new(elem_type), len))
}

fn convert_tuple_type(tuple: &syn::TypeTuple) -> Result<SolType, ConversionError> {
    let mut types = Vec::new();
    for elem in &tuple.elems {
        let elem_type = rust_to_sol(elem)?;
        types.push(elem_type);
    }
    Ok(SolType::Tuple(types))
}

fn convert_slice_type(slice: &syn::TypeSlice) -> Result<SolType, ConversionError> {
    let elem_type = rust_to_sol(&slice.elem)?;
    Ok(SolType::Array(Box::new(elem_type)))
}

fn convert_fixed_bytes(type_name: &str, args: &PathArguments) -> Result<SolType, ConversionError> {
    if let PathArguments::AngleBracketed(angle_args) = args {
        if let Some(GenericArgument::Const(syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(lit_int),
            ..
        }))) = angle_args.args.first()
        {
            if let Ok(size) = lit_int.base10_parse::<usize>() {
                if size > 0 && size <= 32 {
                    return Ok(SolType::FixedBytes(size));
                }
            }
        }
    }
    Err(ConversionError::InvalidBytesSize(format!(
        "{type_name} requires a size parameter between 1 and 32"
    )))
}
#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_str;

    fn assert_type(rust_type: &str, expected: SolType) {
        let ty: Type = parse_str(rust_type).unwrap();
        let result = rust_to_sol(&ty).unwrap();
        assert_eq!(result, expected);
    }

    fn assert_error<F>(rust_type: &str, error_check: F)
    where
        F: FnOnce(&ConversionError),
    {
        let ty: Type = parse_str(rust_type).unwrap();
        let result = rust_to_sol(&ty);
        assert!(result.is_err());
        error_check(&result.unwrap_err());
    }

    #[test]
    fn test_primitive_types() {
        assert_type("bool", SolType::Bool);
        assert_type("Address", SolType::Address);
        assert_type("String", SolType::String);
        assert_type("Bytes", SolType::Bytes);
    }

    #[test]
    fn test_primitive_types_uint() {
        // Unsigned types (aliases and rust-like names)
        assert_type("u8", SolType::Uint(8));
        assert_type("U8", SolType::Uint(8));
        assert_type("u16", SolType::Uint(16));
        assert_type("U16", SolType::Uint(16));
        assert_type("u24", SolType::Uint(24));
        assert_type("U24", SolType::Uint(24));
        assert_type("u32", SolType::Uint(32));
        assert_type("U32", SolType::Uint(32));
        assert_type("u40", SolType::Uint(40));
        assert_type("U40", SolType::Uint(40));
        assert_type("u48", SolType::Uint(48));
        assert_type("U48", SolType::Uint(48));
        assert_type("u56", SolType::Uint(56));
        assert_type("U56", SolType::Uint(56));
        assert_type("u64", SolType::Uint(64));
        assert_type("U64", SolType::Uint(64));
        assert_type("u72", SolType::Uint(72));
        assert_type("U72", SolType::Uint(72));
        assert_type("u80", SolType::Uint(80));
        assert_type("U80", SolType::Uint(80));
        assert_type("u88", SolType::Uint(88));
        assert_type("U88", SolType::Uint(88));
        assert_type("u96", SolType::Uint(96));
        assert_type("U96", SolType::Uint(96));
        assert_type("u104", SolType::Uint(104));
        assert_type("U104", SolType::Uint(104));
        assert_type("u112", SolType::Uint(112));
        assert_type("U112", SolType::Uint(112));
        assert_type("u120", SolType::Uint(120));
        assert_type("U120", SolType::Uint(120));
        assert_type("u128", SolType::Uint(128));
        assert_type("U128", SolType::Uint(128));
        assert_type("u136", SolType::Uint(136));
        assert_type("U136", SolType::Uint(136));
        assert_type("u144", SolType::Uint(144));
        assert_type("U144", SolType::Uint(144));
        assert_type("u152", SolType::Uint(152));
        assert_type("U152", SolType::Uint(152));
        assert_type("u160", SolType::Uint(160));
        assert_type("U160", SolType::Uint(160));
        assert_type("u168", SolType::Uint(168));
        assert_type("U168", SolType::Uint(168));
        assert_type("u176", SolType::Uint(176));
        assert_type("U176", SolType::Uint(176));
        assert_type("u184", SolType::Uint(184));
        assert_type("U184", SolType::Uint(184));
        assert_type("u192", SolType::Uint(192));
        assert_type("U192", SolType::Uint(192));
        assert_type("u200", SolType::Uint(200));
        assert_type("U200", SolType::Uint(200));
        assert_type("u208", SolType::Uint(208));
        assert_type("U208", SolType::Uint(208));
        assert_type("u216", SolType::Uint(216));
        assert_type("U216", SolType::Uint(216));
        assert_type("u224", SolType::Uint(224));
        assert_type("U224", SolType::Uint(224));
        assert_type("u232", SolType::Uint(232));
        assert_type("U232", SolType::Uint(232));
        assert_type("u240", SolType::Uint(240));
        assert_type("U240", SolType::Uint(240));
        assert_type("u248", SolType::Uint(248));
        assert_type("U248", SolType::Uint(248));
        assert_type("u256", SolType::Uint(256));
        assert_type("U256", SolType::Uint(256));
        assert_type("u512", SolType::Uint(512));
        assert_type("U512", SolType::Uint(512));
    }
    #[test]
    fn test_primitive_types_int() {
        // Signed types (aliases and rust-like names)
        assert_type("i8", SolType::Int(8));
        assert_type("I8", SolType::Int(8));
        assert_type("i16", SolType::Int(16));
        assert_type("I16", SolType::Int(16));
        assert_type("i24", SolType::Int(24));
        assert_type("I24", SolType::Int(24));
        assert_type("i32", SolType::Int(32));
        assert_type("I32", SolType::Int(32));
        assert_type("i40", SolType::Int(40));
        assert_type("I40", SolType::Int(40));
        assert_type("i48", SolType::Int(48));
        assert_type("I48", SolType::Int(48));
        assert_type("i56", SolType::Int(56));
        assert_type("I56", SolType::Int(56));
        assert_type("i64", SolType::Int(64));
        assert_type("I64", SolType::Int(64));
        assert_type("i72", SolType::Int(72));
        assert_type("I72", SolType::Int(72));
        assert_type("i80", SolType::Int(80));
        assert_type("I80", SolType::Int(80));
        assert_type("i88", SolType::Int(88));
        assert_type("I88", SolType::Int(88));
        assert_type("i96", SolType::Int(96));
        assert_type("I96", SolType::Int(96));
        assert_type("i104", SolType::Int(104));
        assert_type("I104", SolType::Int(104));
        assert_type("i112", SolType::Int(112));
        assert_type("I112", SolType::Int(112));
        assert_type("i120", SolType::Int(120));
        assert_type("I120", SolType::Int(120));
        assert_type("i128", SolType::Int(128));
        assert_type("I128", SolType::Int(128));
        assert_type("i136", SolType::Int(136));
        assert_type("I136", SolType::Int(136));
        assert_type("i144", SolType::Int(144));
        assert_type("I144", SolType::Int(144));
        assert_type("i152", SolType::Int(152));
        assert_type("I152", SolType::Int(152));
        assert_type("i160", SolType::Int(160));
        assert_type("I160", SolType::Int(160));
        assert_type("i168", SolType::Int(168));
        assert_type("I168", SolType::Int(168));
        assert_type("i176", SolType::Int(176));
        assert_type("I176", SolType::Int(176));
        assert_type("i184", SolType::Int(184));
        assert_type("I184", SolType::Int(184));
        assert_type("i192", SolType::Int(192));
        assert_type("I192", SolType::Int(192));
        assert_type("i200", SolType::Int(200));
        assert_type("I200", SolType::Int(200));
        assert_type("i208", SolType::Int(208));
        assert_type("I208", SolType::Int(208));
        assert_type("i216", SolType::Int(216));
        assert_type("I216", SolType::Int(216));
        assert_type("i224", SolType::Int(224));
        assert_type("I224", SolType::Int(224));
        assert_type("i232", SolType::Int(232));
        assert_type("I232", SolType::Int(232));
        assert_type("i240", SolType::Int(240));
        assert_type("I240", SolType::Int(240));
        assert_type("i248", SolType::Int(248));
        assert_type("I248", SolType::Int(248));
        assert_type("i256", SolType::Int(256));
        assert_type("I256", SolType::Int(256));
        assert_type("i512", SolType::Int(512));
        assert_type("I512", SolType::Int(512));
    }

    #[test]
    fn test_array_types() {
        // Fixed size arrays of u8 up to 32 bytes are now converted to FixedBytes
        assert_type("[u8; 5]", SolType::FixedBytes(5));
        assert_type("[u8; 32]", SolType::FixedBytes(32));

        // Other fixed size arrays remain as FixedArray
        assert_type(
            "[bool; 10]",
            SolType::FixedArray(Box::new(SolType::Bool), 10),
        );
        assert_type(
            "[Address; 3]",
            SolType::FixedArray(Box::new(SolType::Address), 3),
        );

        // Nested arrays - now with updated inner type
        assert_type(
            "[[u8; 5]; 3]",
            SolType::FixedArray(Box::new(SolType::FixedBytes(5)), 3),
        );

        // Vec with fixed size arrays - now with updated inner type
        assert_type(
            "Vec<[u8; 5]>",
            SolType::Array(Box::new(SolType::FixedBytes(5))),
        );

        // Dynamic arrays
        assert_type("Vec<u8>", SolType::Array(Box::new(SolType::Uint(8))));
        assert_type(
            "Vec<Vec<bool>>",
            SolType::Array(Box::new(SolType::Array(Box::new(SolType::Bool)))),
        );
    }

    #[test]
    fn test_u8_arrays_to_fixed_bytes() {
        // Test various sizes to ensure conversion is working
        assert_type("[u8; 1]", SolType::FixedBytes(1));
        assert_type("[u8; 16]", SolType::FixedBytes(16));
        assert_type("[u8; 32]", SolType::FixedBytes(32));

        // Test that arrays larger than 32 remain as FixedArray
        assert_type(
            "[u8; 33]",
            SolType::FixedArray(Box::new(SolType::Uint(8)), 33),
        );

        // Test that arrays of other types remain as FixedArray, even if length <= 32
        assert_type(
            "[u16; 32]",
            SolType::FixedArray(Box::new(SolType::Uint(16)), 32),
        );
    }

    #[test]
    fn test_invalid_arrays() {
        // Zero length arrays
        assert_error("[u8; 0]", |e| {
            assert!(matches!(e, ConversionError::InvalidArrayLength(_)));
        });

        // Non-literal length
        assert_error("[u8; invalid]", |e| {
            assert!(matches!(e, ConversionError::InvalidArrayLength(_)));
        });
    }

    #[test]
    fn test_tuple_types() {
        assert_type("()", SolType::Tuple(vec![]));
        assert_type(
            "(u8, bool)",
            SolType::Tuple(vec![SolType::Uint(8), SolType::Bool]),
        );
        assert_type(
            "(u8, (bool, Address))",
            SolType::Tuple(vec![
                SolType::Uint(8),
                SolType::Tuple(vec![SolType::Bool, SolType::Address]),
            ]),
        );
    }

    #[test]
    fn test_fixed_bytes() {
        assert_type("FixedBytes<1>", SolType::FixedBytes(1));
        assert_type("FixedBytes<32>", SolType::FixedBytes(32));

        // Invalid sizes
        assert_error("FixedBytes<0>", |e| {
            assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
        });
        assert_error("FixedBytes<33>", |e| {
            assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
        });
    }

    #[test]
    fn test_references() {
        assert_type("&u8", SolType::Uint(8));
        assert_type("&mut bool", SolType::Bool);
        assert_type("&Vec<u8>", SolType::Array(Box::new(SolType::Uint(8))));

        // Fixed size array of u8 is now treated as FixedBytes
        assert_type("&[u8; 5]", SolType::FixedBytes(5));
    }

    #[test]
    fn test_custom_types() {
        assert_type(
            "MyStruct",
            SolType::Struct {
                name: "MyStruct".to_string(),
                fields: vec![],
            },
        );

        assert_type(
            "types::MyStruct",
            SolType::Struct {
                name: "types::MyStruct".to_string(),
                fields: vec![],
            },
        );

        // Invalid generic types
        assert_error("MyStruct<T>", |e| {
            assert!(matches!(e, ConversionError::UnsupportedType(_)));
        });
        assert_error("Container<K, V>", |e| {
            assert!(matches!(e, ConversionError::UnsupportedType(_)));
        });
    }
}

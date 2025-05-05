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
    match type_name {
        // Unsigned integers
        name @ ("u8" | "u16" | "u32" | "u64" | "u128") => {
            let bits = name[1..].parse::<usize>().ok()?;
            Some(SolType::Uint(bits))
        }
        "u256" | "U256" => Some(SolType::Uint(256)),

        // Signed integers
        name @ ("i8" | "i16" | "i32" | "i64" | "i128") => {
            let bits = name[1..].parse::<usize>().ok()?;
            Some(SolType::Int(bits))
        }
        "i256" | "I256" => Some(SolType::Int(256)),

        // Other primitive types
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
        assert_type("u8", SolType::Uint(8));
        assert_type("u16", SolType::Uint(16));
        assert_type("u32", SolType::Uint(32));
        assert_type("u64", SolType::Uint(64));
        assert_type("u128", SolType::Uint(128));
        assert_type("U256", SolType::Uint(256));
        assert_type("i8", SolType::Int(8));
        assert_type("i256", SolType::Int(256));
        assert_type("Address", SolType::Address);
        assert_type("String", SolType::String);
        assert_type("Bytes", SolType::Bytes);
    }

    #[test]
    fn test_array_types() {
        // Fixed size arrays
        assert_type(
            "[u8; 5]",
            SolType::FixedArray(Box::new(SolType::Uint(8)), 5),
        );
        assert_type(
            "[bool; 10]",
            SolType::FixedArray(Box::new(SolType::Bool), 10),
        );
        assert_type(
            "[Address; 3]",
            SolType::FixedArray(Box::new(SolType::Address), 3),
        );

        // Nested arrays
        assert_type(
            "[[u8; 5]; 3]",
            SolType::FixedArray(
                Box::new(SolType::FixedArray(Box::new(SolType::Uint(8)), 5)),
                3,
            ),
        );

        // Vec with fixed size arrays
        assert_type(
            "Vec<[u8; 5]>",
            SolType::Array(Box::new(SolType::FixedArray(Box::new(SolType::Uint(8)), 5))),
        );

        // Dynamic arrays
        assert_type("Vec<u8>", SolType::Array(Box::new(SolType::Uint(8))));
        assert_type(
            "Vec<Vec<bool>>",
            SolType::Array(Box::new(SolType::Array(Box::new(SolType::Bool)))),
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
        assert_type(
            "&[u8; 5]",
            SolType::FixedArray(Box::new(SolType::Uint(8)), 5),
        );
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

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

    // A `B<bits>` alias is decided before the primitive table, so that a width the Solidity ABI
    // has no type for is rejected rather than silently mapped to something else.
    if let Some(width) = fixed_bytes_alias_width(&type_name) {
        return fixed_bytes(width, &type_name);
    }

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
    const ALLOWED: [usize; 33] = [
        8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168,
        176, 184, 192, 200, 208, 216, 224, 232, 240, 248, 256, 512,
    ];

    // Unsigned types (u8, U8, ...)
    if type_name.starts_with('u') || type_name.starts_with('U') {
        if let Ok(bits) = type_name[1..].parse::<usize>() {
            if ALLOWED.contains(&bits) {
                return Some(SolType::Uint(bits));
            }
        }
    }

    // Signed types (i8, I8, ...)
    if type_name.starts_with('i') || type_name.starts_with('I') {
        if let Ok(bits) = type_name[1..].parse::<usize>() {
            if ALLOWED.contains(&bits) {
                return Some(SolType::Int(bits));
            }
        }
    }

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

    // `[u8; N]` is `uint8[N]`, not `bytesN`. The codec encodes a Rust array element by element -
    // one word per `u8` - so a selector that said `bytesN` would accept canonical calldata the
    // router cannot decode. `bytesN` is what `FixedBytes<N>` and the `B8`..`B256` aliases carry.
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

/// The width in bytes a `B<bits>` alias stands for, if the name is one.
///
/// The list is exactly the aliases the primitive types crate defines; any other `B`-prefixed name
/// is a user's own type - `B24` and `B4096` are structs, not fixed bytes. Whether a listed width
/// has a Solidity type is [`fixed_bytes`]'s decision, not this one: every alias is routed there,
/// so a wide one cannot reach the primitive table and bypass the width rule.
fn fixed_bytes_alias_width(type_name: &str) -> Option<usize> {
    const ALIASES: [(&str, usize); 13] = [
        ("B8", 1),
        ("B16", 2),
        ("B32", 4),
        ("B64", 8),
        ("B96", 12),
        ("B128", 16),
        ("B160", 20),
        ("B192", 24),
        ("B224", 28),
        ("B256", 32),
        ("B512", 64),
        ("B1024", 128),
        ("B2048", 256),
    ];

    ALIASES
        .iter()
        .find(|(name, _)| *name == type_name)
        .map(|(_, width)| *width)
}

/// The single home of "the Solidity ABI has no fixed-bytes type wider than `bytes32`".
///
/// Both spellings of a fixed-bytes parameter land here - the `B<bits>` aliases and an explicit
/// `FixedBytes<N>` - so they are accepted and refused on the same rule, with the same message.
/// `fluentbase-codec` asserts the same bound at the type level for the Solidity ABI.
fn fixed_bytes(width: usize, described_as: &str) -> Result<SolType, ConversionError> {
    // The two ways to be outside `bytes1`..`bytes32` fail for different reasons, so they say
    // different things: there is no zero-width Solidity type at all, while a too-wide one has
    // `Bytes` and `[u8; N]` to fall back on.
    if width == 0 {
        return Err(ConversionError::InvalidBytesSize(format!(
            "{described_as} is zero bytes wide; the Solidity ABI's fixed-bytes types start at \
             bytes1"
        )));
    }
    if width > 32 {
        return Err(ConversionError::InvalidBytesSize(format!(
            "{described_as} is {width} bytes wide; the Solidity ABI has no fixed-bytes type wider \
             than bytes32 - use `Bytes` for a dynamic blob or `[u8; {width}]` for \
             `uint8[{width}]`"
        )));
    }
    Ok(SolType::FixedBytes(width))
}

fn convert_fixed_bytes(type_name: &str, args: &PathArguments) -> Result<SolType, ConversionError> {
    if let PathArguments::AngleBracketed(angle_args) = args {
        if let Some(GenericArgument::Const(syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(lit_int),
            ..
        }))) = angle_args.args.first()
        {
            if let Ok(size) = lit_int.base10_parse::<usize>() {
                return fixed_bytes(size, &format!("{type_name}<{size}>"));
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
        // Fixed-size arrays of u8 are `uint8[N]`, never `bytesN`: the codec lays them out one
        // word per element, and the selector has to describe that layout
        assert_type(
            "[u8; 5]",
            SolType::FixedArray(Box::new(SolType::Uint(8)), 5),
        );
        assert_type(
            "[u8; 32]",
            SolType::FixedArray(Box::new(SolType::Uint(8)), 32),
        );

        // Other fixed size arrays remain as FixedArray
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
    fn test_u8_arrays_are_uint8_arrays_at_every_length() {
        // The mapping does not change at 32: `bytesN` is a different layout (one right-padded
        // word) and is only reachable through `FixedBytes<N>` and the `B*` aliases
        for len in [1usize, 16, 32, 33] {
            assert_type(
                &format!("[u8; {len}]"),
                SolType::FixedArray(Box::new(SolType::Uint(8)), len),
            );
        }

        // Arrays of other element types are fixed arrays as well
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

        // Invalid sizes. The two ends fail for different reasons, and each message has to name
        // its own: a zero-width type has no Solidity counterpart at all, while a too-wide one is
        // pointed at `Bytes` and `[u8; N]`.
        assert_error("FixedBytes<0>", |e| {
            let ConversionError::InvalidBytesSize(msg) = e else {
                panic!("expected InvalidBytesSize, got {e:?}");
            };
            assert!(msg.contains("zero bytes wide"), "{msg}");
            assert!(msg.contains("start at bytes1"), "{msg}");
        });
        assert_error("FixedBytes<33>", |e| {
            let ConversionError::InvalidBytesSize(msg) = e else {
                panic!("expected InvalidBytesSize, got {e:?}");
            };
            assert!(msg.contains("33 bytes wide"), "{msg}");
            assert!(msg.contains("wider than bytes32"), "{msg}");
        });
    }

    #[test]
    fn test_references() {
        assert_type("&u8", SolType::Uint(8));
        assert_type("&mut bool", SolType::Bool);
        assert_type("&Vec<u8>", SolType::Array(Box::new(SolType::Uint(8))));

        // A reference does not change the mapping of a byte array either
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

    mod b_type_tests {
        use super::*;
        use syn::parse_str;

        fn assert_type(rust_type: &str, expected: SolType) {
            let ty: Type = parse_str(rust_type).unwrap();
            let result = rust_to_sol(&ty).unwrap();
            assert_eq!(result, expected);
        }

        fn assert_struct_type(rust_type: &str) {
            let ty: Type = parse_str(rust_type).unwrap();
            let result = rust_to_sol(&ty).unwrap();
            // Verify that this is treated as a struct, not as FixedBytes
            assert!(matches!(result, SolType::Struct { .. }));
        }

        #[test]
        fn test_exact_b_types() {
            // Test all supported B-types that convert to Solidity bytesN
            assert_type("B8", SolType::FixedBytes(1)); // 8 bits = 1 byte
            assert_type("B16", SolType::FixedBytes(2)); // 16 bits = 2 bytes
            assert_type("B32", SolType::FixedBytes(4)); // 32 bits = 4 bytes
            assert_type("B64", SolType::FixedBytes(8)); // 64 bits = 8 bytes
            assert_type("B96", SolType::FixedBytes(12)); // 96 bits = 12 bytes
            assert_type("B128", SolType::FixedBytes(16)); // 128 bits = 16 bytes
            assert_type("B160", SolType::FixedBytes(20)); // 160 bits = 20 bytes (address size)
            assert_type("B192", SolType::FixedBytes(24)); // 192 bits = 24 bytes
            assert_type("B224", SolType::FixedBytes(28)); // 224 bits = 28 bytes
            assert_type("B256", SolType::FixedBytes(32)); // 256 bits = 32 bytes (hash size)
        }

        #[test]
        fn test_wide_b_types_have_no_solidity_type() {
            // Solidity's fixed-bytes types stop at `bytes32`. These aliases used to be advertised
            // as `uint8[N]`, which is not the single inline blob the codec writes for them, so no
            // selector could ever match the calldata; they are rejected instead
            for ty in ["B512", "B1024", "B2048"] {
                assert_error(ty, |e| {
                    assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
                });
            }
        }

        #[test]
        fn test_b_like_structs_not_converted() {
            // Test that types starting with 'B' but not in our exact list
            // are correctly treated as custom structs, not as B-types
            assert_struct_type("BankAccount"); // Common struct name starting with B
            assert_struct_type("Buffer"); // Another common struct name
            assert_struct_type("BlockHeader"); // Blockchain-related struct
            assert_struct_type("B7"); // Not in our supported list
            assert_struct_type("B24"); // Not in our supported list
            assert_struct_type("B100"); // Not in our supported list
            assert_struct_type("B4096"); // Not in our supported list
            assert_struct_type("BigNumber"); // Should not be confused with B-types
        }

        #[test]
        fn test_b_types_in_collections() {
            // Test that B-types work correctly inside collections

            // Dynamic arrays (Vec)
            assert_type(
                "Vec<B256>",
                SolType::Array(Box::new(SolType::FixedBytes(32))),
            );
            assert_type(
                "Vec<B128>",
                SolType::Array(Box::new(SolType::FixedBytes(16))),
            );

            // Fixed arrays
            assert_type(
                "[B128; 5]",
                SolType::FixedArray(Box::new(SolType::FixedBytes(16)), 5),
            );
            assert_type(
                "[B64; 10]",
                SolType::FixedArray(Box::new(SolType::FixedBytes(8)), 10),
            );

            // A wide alias is rejected inside a collection as well
            assert_error("Vec<B512>", |e| {
                assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
            });

            // Nested collections
            assert_type(
                "Vec<Vec<B256>>",
                SolType::Array(Box::new(SolType::Array(Box::new(SolType::FixedBytes(32))))),
            );
        }

        #[test]
        fn test_b_types_in_tuples() {
            // Test B-types in tuple combinations
            assert_type(
                "(B256, B128)",
                SolType::Tuple(vec![SolType::FixedBytes(32), SolType::FixedBytes(16)]),
            );

            // Mixed with other types
            assert_type(
                "(u256, B256, bool)",
                SolType::Tuple(vec![
                    SolType::Uint(256),
                    SolType::FixedBytes(32),
                    SolType::Bool,
                ]),
            );

            // A wide alias is rejected inside a tuple as well
            assert_error("(B512, u64)", |e| {
                assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
            });
        }

        #[test]
        fn test_mixed_b_types_and_structs() {
            // Test mixing real B-types with B-like struct names
            assert_type(
                "(B256, BankAccount, B128)",
                SolType::Tuple(vec![
                    SolType::FixedBytes(32), // Real B-type
                    SolType::Struct {
                        // Custom struct
                        name: "BankAccount".to_string(),
                        fields: vec![],
                    },
                    SolType::FixedBytes(16), // Real B-type
                ]),
            );
        }

        #[test]
        fn test_b_types_with_references() {
            // Test that references to B-types work correctly
            assert_type("&B256", SolType::FixedBytes(32));
            assert_type("&mut B128", SolType::FixedBytes(16));
            assert_type(
                "&[B64; 5]",
                SolType::FixedArray(Box::new(SolType::FixedBytes(8)), 5),
            );

            // A reference to a wide alias is rejected as well
            assert_error("&B512", |e| {
                assert!(matches!(e, ConversionError::InvalidBytesSize(_)));
            });
        }

        #[test]
        fn test_common_use_cases() {
            // Test common blockchain/crypto use cases for B-types

            // Hash types
            assert_type("B256", SolType::FixedBytes(32)); // Common for SHA256, Keccak256
            assert_type("B160", SolType::FixedBytes(20)); // Ethereum address size

            // Signature components
            assert_type(
                "(B256, B256, u8)",
                SolType::Tuple(vec![
                    SolType::FixedBytes(32), // r
                    SolType::FixedBytes(32), // s
                    SolType::Uint(8),        // v
                ]),
            );

            // Array of hashes
            assert_type(
                "Vec<B256>",
                SolType::Array(Box::new(SolType::FixedBytes(32))),
            );

            // Merkle proof
            assert_type(
                "[B256; 10]",
                SolType::FixedArray(Box::new(SolType::FixedBytes(32)), 10),
            );
        }
    }
}

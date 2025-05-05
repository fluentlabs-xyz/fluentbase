use crate::abi::{error::ABIError, types::sol::SolType};
use syn::{parse_quote, Type};

/// Converts a Solidity type to a corresponding Rust type
pub fn sol_to_rust(sol_type: &SolType) -> Result<Type, ABIError> {
    match sol_type {
        // Primitive types with direct mappings
        SolType::Bool => Ok(parse_quote!(bool)),
        SolType::String => Ok(parse_quote!(String)),
        SolType::Address => Ok(parse_quote!(Address)),
        SolType::Bytes => Ok(parse_quote!(Bytes)),

        // Integer types
        SolType::Uint(bits) => to_rust_uint(*bits),
        SolType::Int(bits) => to_rust_int(*bits),

        // Fixed-size types
        SolType::FixedBytes(size) => to_rust_fixed_bytes(*size),

        // Container types
        SolType::Array(inner) => {
            let inner_type = sol_to_rust(inner)?;
            Ok(parse_quote!(Vec<#inner_type>))
        }
        SolType::FixedArray(inner, size) => {
            let inner_type = sol_to_rust(inner)?;
            Ok(parse_quote!([#inner_type; #size]))
        }
        SolType::Tuple(types) => to_rust_tuple(types),

        // User-defined types
        SolType::Struct { name, .. } => Ok(parse_quote!(#name)),
    }
}

// Integer conversion with standard sizes
fn to_rust_uint(bits: usize) -> Result<Type, ABIError> {
    match bits {
        8 => Ok(parse_quote!(u8)),
        16 => Ok(parse_quote!(u16)),
        32 => Ok(parse_quote!(u32)),
        64 => Ok(parse_quote!(u64)),
        128 => Ok(parse_quote!(u128)),
        256 => Ok(parse_quote!(U256)),
        _ => Err(ABIError::UnsupportedType(format!("uint{bits}"))),
    }
}

fn to_rust_int(bits: usize) -> Result<Type, ABIError> {
    match bits {
        8 => Ok(parse_quote!(i8)),
        16 => Ok(parse_quote!(i16)),
        32 => Ok(parse_quote!(i32)),
        64 => Ok(parse_quote!(i64)),
        128 => Ok(parse_quote!(i128)),
        256 => Ok(parse_quote!(I256)),
        _ => Err(ABIError::UnsupportedType(format!("int{bits}"))),
    }
}

fn to_rust_fixed_bytes(size: usize) -> Result<Type, ABIError> {
    if !(1..=32).contains(&size) {
        return Err(ABIError::UnsupportedType(format!(
            "FixedBytes size must be between 1 and 32, got {size}"
        )));
    }
    Ok(parse_quote!(FixedBytes<#size>))
}

fn to_rust_tuple(types: &[SolType]) -> Result<Type, ABIError> {
    match types.len() {
        0 => Ok(parse_quote!(())),
        1 => sol_to_rust(&types[0]),
        _ => {
            let rust_types: Result<Vec<_>, _> = types.iter().map(sol_to_rust).collect();
            let rust_types = rust_types?;
            Ok(parse_quote!((#(#rust_types),*)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn assert_type_eq(sol_type: SolType, expected: &str) {
        let rust_type = sol_to_rust(&sol_type).unwrap();
        assert_eq!(
            rust_type.to_token_stream().to_string().replace(' ', ""),
            expected.replace(' ', "")
        );
    }

    #[test]
    fn test_primitive_conversions() {
        let test_cases = [
            (SolType::Bool, "bool"),
            (SolType::String, "String"),
            (SolType::Address, "Address"),
            (SolType::Bytes, "Bytes"),
            (SolType::Uint(8), "u8"),
            (SolType::Uint(256), "U256"),
            (SolType::Int(128), "i128"),
        ];

        for (sol_type, expected) in test_cases {
            assert_type_eq(sol_type, expected);
        }
    }

    // #[test]
    // fn test_container_types() {
    //     let test_cases = [
    //         (SolType::Array(Box::new(SolType::Uint(256))), "Vec<U256>"),
    //         (SolType::FixedArray(Box::new(SolType::Bool), 5), "[bool; 5]"),
    //         (
    //             SolType::Tuple(vec![SolType::Uint(8), SolType::Bool]),
    //             "(u8, bool)",
    //         ),
    //     ];

    //     for (sol_type, expected) in test_cases {
    //         assert_type_eq(sol_type, expected);
    //     }
    // }

    // #[test]
    // fn test_fixed_bytes() {
    //     assert_type_eq(SolType::FixedBytes(32), "FixedBytes<32>");
    //     assert!(sol_to_rust(&SolType::FixedBytes(33)).is_err());
    //     assert!(sol_to_rust(&SolType::FixedBytes(0)).is_err());
    // }

    #[test]
    fn test_tuples() {
        // Empty tuple
        assert_type_eq(SolType::Tuple(vec![]), "()");

        // Single element (unwrapped)
        assert_type_eq(SolType::Tuple(vec![SolType::Bool]), "bool");

        // Multiple elements
        assert_type_eq(
            SolType::Tuple(vec![SolType::Uint(8), SolType::Bool, SolType::String]),
            "(u8, bool, String)",
        );
    }

    // #[test]
    // fn test_struct_conversion() {
    //     assert_type_eq(
    //         SolType::Struct {
    //             name: "MyStruct".into(),
    //             fields: vec![],
    //         },
    //         "MyStruct",
    //     );
    // }

    #[test]
    fn test_invalid_types() {
        assert!(sol_to_rust(&SolType::Uint(7)).is_err());
        assert!(sol_to_rust(&SolType::Int(512)).is_err());
    }
}

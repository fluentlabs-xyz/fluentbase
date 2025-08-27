use crate::abi::{error::ABIError, types::sol::SolType};
use quote::format_ident;
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
        SolType::Struct { name, .. } => {
            let ident = format_ident!("{}", name);
            Ok(parse_quote!(#ident))
        }
    }
}

fn to_rust_uint(bits: usize) -> Result<Type, ABIError> {
    match bits {
        8 => Ok(parse_quote!(u8)),
        16 => Ok(parse_quote!(u16)),
        24 => Ok(parse_quote!(U24)),
        32 => Ok(parse_quote!(u32)),
        40 => Ok(parse_quote!(U40)),
        48 => Ok(parse_quote!(U48)),
        56 => Ok(parse_quote!(U56)),
        64 => Ok(parse_quote!(u64)),
        72 => Ok(parse_quote!(U72)),
        80 => Ok(parse_quote!(U80)),
        88 => Ok(parse_quote!(U88)),
        96 => Ok(parse_quote!(U96)),
        104 => Ok(parse_quote!(U104)),
        112 => Ok(parse_quote!(U112)),
        120 => Ok(parse_quote!(U120)),
        128 => Ok(parse_quote!(u128)),
        136 => Ok(parse_quote!(U136)),
        144 => Ok(parse_quote!(U144)),
        152 => Ok(parse_quote!(U152)),
        160 => Ok(parse_quote!(U160)),
        168 => Ok(parse_quote!(U168)),
        176 => Ok(parse_quote!(U176)),
        184 => Ok(parse_quote!(U184)),
        192 => Ok(parse_quote!(U192)),
        200 => Ok(parse_quote!(U200)),
        208 => Ok(parse_quote!(U208)),
        216 => Ok(parse_quote!(U216)),
        224 => Ok(parse_quote!(U224)),
        232 => Ok(parse_quote!(U232)),
        240 => Ok(parse_quote!(U240)),
        248 => Ok(parse_quote!(U248)),
        256 => Ok(parse_quote!(U256)),
        512 => Ok(parse_quote!(U512)),
        _ => Err(ABIError::UnsupportedType(format!("uint{bits}").into())),
    }
}

fn to_rust_int(bits: usize) -> Result<Type, ABIError> {
    match bits {
        8 => Ok(parse_quote!(i8)),
        16 => Ok(parse_quote!(i16)),
        24 => Ok(parse_quote!(I24)),
        32 => Ok(parse_quote!(i32)),
        40 => Ok(parse_quote!(I40)),
        48 => Ok(parse_quote!(I48)),
        56 => Ok(parse_quote!(I56)),
        64 => Ok(parse_quote!(i64)),
        72 => Ok(parse_quote!(I72)),
        80 => Ok(parse_quote!(I80)),
        88 => Ok(parse_quote!(I88)),
        96 => Ok(parse_quote!(I96)),
        104 => Ok(parse_quote!(I104)),
        112 => Ok(parse_quote!(I112)),
        120 => Ok(parse_quote!(I120)),
        128 => Ok(parse_quote!(i128)),
        136 => Ok(parse_quote!(I136)),
        144 => Ok(parse_quote!(I144)),
        152 => Ok(parse_quote!(I152)),
        160 => Ok(parse_quote!(I160)),
        168 => Ok(parse_quote!(I168)),
        176 => Ok(parse_quote!(I176)),
        184 => Ok(parse_quote!(I184)),
        192 => Ok(parse_quote!(I192)),
        200 => Ok(parse_quote!(I200)),
        208 => Ok(parse_quote!(I208)),
        216 => Ok(parse_quote!(I216)),
        224 => Ok(parse_quote!(I224)),
        232 => Ok(parse_quote!(I232)),
        240 => Ok(parse_quote!(I240)),
        248 => Ok(parse_quote!(I248)),
        256 => Ok(parse_quote!(I256)),
        512 => Ok(parse_quote!(I512)),
        _ => Err(ABIError::UnsupportedType(format!("int{bits}").into())),
    }
}

fn to_rust_fixed_bytes(size: usize) -> Result<Type, ABIError> {
    if !(1..=32).contains(&size) {
        return Err(ABIError::UnsupportedType(format!(
            "FixedBytes size must be between 1 and 32, got {size}"
        ).into()));
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
            // Unsigned types
            (SolType::Uint(8), "u8"),
            (SolType::Uint(16), "u16"),
            (SolType::Uint(24), "U24"),
            (SolType::Uint(32), "u32"),
            (SolType::Uint(40), "U40"),
            (SolType::Uint(48), "U48"),
            (SolType::Uint(56), "U56"),
            (SolType::Uint(64), "u64"),
            (SolType::Uint(72), "U72"),
            (SolType::Uint(80), "U80"),
            (SolType::Uint(88), "U88"),
            (SolType::Uint(96), "U96"),
            (SolType::Uint(104), "U104"),
            (SolType::Uint(112), "U112"),
            (SolType::Uint(120), "U120"),
            (SolType::Uint(128), "u128"),
            (SolType::Uint(136), "U136"),
            (SolType::Uint(144), "U144"),
            (SolType::Uint(152), "U152"),
            (SolType::Uint(160), "U160"),
            (SolType::Uint(168), "U168"),
            (SolType::Uint(176), "U176"),
            (SolType::Uint(184), "U184"),
            (SolType::Uint(192), "U192"),
            (SolType::Uint(200), "U200"),
            (SolType::Uint(208), "U208"),
            (SolType::Uint(216), "U216"),
            (SolType::Uint(224), "U224"),
            (SolType::Uint(232), "U232"),
            (SolType::Uint(240), "U240"),
            (SolType::Uint(248), "U248"),
            (SolType::Uint(256), "U256"),
            (SolType::Uint(512), "U512"),
            // Signed types
            (SolType::Int(8), "i8"),
            (SolType::Int(16), "i16"),
            (SolType::Int(24), "I24"),
            (SolType::Int(32), "i32"),
            (SolType::Int(40), "I40"),
            (SolType::Int(48), "I48"),
            (SolType::Int(56), "I56"),
            (SolType::Int(64), "i64"),
            (SolType::Int(72), "I72"),
            (SolType::Int(80), "I80"),
            (SolType::Int(88), "I88"),
            (SolType::Int(96), "I96"),
            (SolType::Int(104), "I104"),
            (SolType::Int(112), "I112"),
            (SolType::Int(120), "I120"),
            (SolType::Int(128), "i128"),
            (SolType::Int(136), "I136"),
            (SolType::Int(144), "I144"),
            (SolType::Int(152), "I152"),
            (SolType::Int(160), "I160"),
            (SolType::Int(168), "I168"),
            (SolType::Int(176), "I176"),
            (SolType::Int(184), "I184"),
            (SolType::Int(192), "I192"),
            (SolType::Int(200), "I200"),
            (SolType::Int(208), "I208"),
            (SolType::Int(216), "I216"),
            (SolType::Int(224), "I224"),
            (SolType::Int(232), "I232"),
            (SolType::Int(240), "I240"),
            (SolType::Int(248), "I248"),
            (SolType::Int(256), "I256"),
            (SolType::Int(512), "I512"),
        ];

        for (sol_type, expected) in test_cases {
            assert_type_eq(sol_type, expected);
        }
    }

    #[test]
    fn test_container_types() {
        let test_cases = [
            (SolType::Array(Box::new(SolType::Uint(256))), "Vec<U256>"),
            (
                SolType::FixedArray(Box::new(SolType::Bool), 5),
                "[bool; 5usize]",
            ),
            (
                SolType::Tuple(vec![SolType::Uint(8), SolType::Bool]),
                "(u8, bool)",
            ),
        ];

        for (sol_type, expected) in test_cases {
            assert_type_eq(sol_type, expected);
        }
    }

    #[test]
    fn test_fixed_bytes() {
        assert_type_eq(SolType::FixedBytes(32), "FixedBytes<32usize>");
        assert!(sol_to_rust(&SolType::FixedBytes(33)).is_err());
        assert!(sol_to_rust(&SolType::FixedBytes(0)).is_err());
    }

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

    #[test]
    fn test_struct_conversion() {
        assert_type_eq(
            SolType::Struct {
                name: "MyStruct".into(),
                fields: vec![],
            },
            "MyStruct",
        );
    }

    #[test]
    fn test_invalid_types() {
        assert!(sol_to_rust(&SolType::Uint(7)).is_err());
        assert!(sol_to_rust(&SolType::Int(513)).is_err());
    }
}

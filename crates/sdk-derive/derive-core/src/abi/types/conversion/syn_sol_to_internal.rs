use crate::abi::{error::ABIError, types::SolType};
use syn_solidity::{Expr, Lit, Type as SolidityAstType};

/// Converts `syn_solidity::Type` to internal `SolType`.
pub fn convert_solidity_type(sol_ty: &SolidityAstType) -> Result<SolType, ABIError> {
    use SolidityAstType::*;

    match sol_ty {
        Address(_, _) => Ok(SolType::Address),
        Bool(_) => Ok(SolType::Bool),
        String(_) => Ok(SolType::String),
        Bytes(_) => Ok(SolType::Bytes),

        FixedBytes(_, size) => Ok(SolType::FixedBytes(size.get() as usize)),

        Uint(_, size) => Ok(SolType::Uint(size.map_or(256, |s| s.get() as usize))),
        Int(_, size) => Ok(SolType::Int(size.map_or(256, |s| s.get() as usize))),

        Array(arr) => {
            let inner = convert_solidity_type(&arr.ty)?;
            match &arr.size {
                Some(expr) => match expr.as_ref() {
                    Expr::Lit(Lit::Number(lit_num)) => {
                        let size = lit_num.base10_parse::<usize>().map_err(|_| {
                            ABIError::UnsupportedType(format!(
                                "Invalid array size: {:?}",
                                lit_num.base10_digits()
                            ).into())
                        })?;
                        Ok(SolType::FixedArray(Box::new(inner), size))
                    }
                    _ => Err(ABIError::UnsupportedType(
                        "Expected numeric literal for array size".into(),
                    )),
                },
                None => Ok(SolType::Array(Box::new(inner))),
            }
        }

        Tuple(tuple) => {
            let types = tuple
                .types
                .iter()
                .map(convert_solidity_type)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SolType::Tuple(types))
        }

        Custom(path) => Ok(SolType::Struct {
            name: path.to_string(),
            fields: vec![],
        }),

        Mapping(_) | Function(_) => Err(ABIError::UnsupportedType(
            "mapping/function not supported".into(),
        )),
    }
}

use core::str::FromStr;
use std::fmt;
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    Expr,
    ExprLit,
    Lit,
    Meta,
    Result,
};
use tracing::warn;

/// List of supported router modes
const VALID_MODES: &[&str] = &["solidity", "fluent"];

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RouterMode {
    Solidity,
    Fluent,
}

impl Default for RouterMode {
    fn default() -> Self {
        Self::Solidity
    }
}

impl FromStr for RouterMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mode = s.to_lowercase();
        match mode.as_str() {
            "solidity" => Ok(Self::Solidity),
            "fluent" => Ok(Self::Fluent),
            invalid => {
                warn!("Invalid router mode: {}", invalid);
                Err(format!(
                    "'{}' is not a valid router mode. Valid modes are: {}",
                    invalid,
                    VALID_MODES.join(", ")
                ))
            }
        }
    }
}

impl Parse for RouterMode {
    fn parse(input: ParseStream) -> Result<Self> {
        let meta = input.parse::<Meta>()?;

        match meta {
            Meta::NameValue(m) if m.path.is_ident("mode") => {
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(lit_str),
                    ..
                }) = m.value
                {
                    FromStr::from_str(&lit_str.value())
                        .map_err(|err| syn::Error::new(lit_str.span(), err))
                } else {
                    Err(syn::Error::new(m.value.span(), "Expected a string literal"))
                }
            }
            _ => Err(syn::Error::new(
                meta.span(),
                "Expected #[router(mode = \"...\")]. Valid modes are: solidity, fluent",
            )),
        }
    }
}

impl fmt::Display for RouterMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solidity => write!(f, "solidity"),
            Self::Fluent => write!(f, "fluent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_mode_from_str() {
        for valid_input in ["solidity", "SOLIDITY", "SoLiDiTy", "Solidity"] {
            assert!(matches!(
                valid_input.parse::<RouterMode>(),
                Ok(RouterMode::Solidity)
            ));
        }

        for valid_input in ["fluent", "FLUENT", "FluENT", "Fluent"] {
            assert!(matches!(
                valid_input.parse::<RouterMode>(),
                Ok(RouterMode::Fluent)
            ));
        }

        for invalid_input in ["invalid", "INVALID"] {
            assert!(invalid_input.parse::<RouterMode>().is_err());
        }
    }
}

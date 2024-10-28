use crate::{error::RouterError, mode::RouterMode};
use proc_macro2::TokenStream as TokenStream2;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Expr,
    ExprLit,
    Lit,
    Meta,
    Result,
    Token,
};
use tracing::{debug, warn};

/// Arguments for router configuration
#[derive(Debug)]
pub struct RouterArgs {
    /// The routing mode to use
    pub(crate) mode: RouterMode,
}

impl RouterArgs {
    /// Creates a new builder for RouterArgs
    pub fn builder() -> RouterArgsBuilder {
        RouterArgsBuilder::default()
    }

    /// Creates a new RouterArgs instance directly
    pub fn new(mode: RouterMode) -> Self {
        Self { mode }
    }

    /// Extracts RouterArgs from a Meta item
    fn from_meta(meta: &Meta) -> Result<Option<Self>> {
        if let Meta::NameValue(m) = meta {
            if m.path.is_ident("mode") {
                debug!("Found mode attribute");
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(lit_str),
                    ..
                }) = &m.value
                {
                    let mode_str = lit_str.value().to_lowercase();
                    return Ok(Some(RouterArgs {
                        mode: mode_str.parse().map_err(|e| {
                            warn!("Invalid mode value: {}", e);
                            e
                        })?,
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Gets the configured mode
    pub fn mode(&self) -> RouterMode {
        self.mode
    }
}

/// Builder for RouterArgs
#[derive(Default)]
pub struct RouterArgsBuilder {
    mode: Option<RouterMode>,
}

impl RouterArgsBuilder {
    /// Sets the routing mode
    pub fn mode(mut self, mode: RouterMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Builds the RouterArgs instance
    pub fn build(self) -> Result<RouterArgs> {
        let mode = self.mode.ok_or_else(|| {
            warn!("Attempted to build RouterArgs without mode");
            RouterError::InvalidMode("Mode must be specified".to_string())
        })?;

        Ok(RouterArgs { mode })
    }
}

impl Parse for RouterArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        debug!("Parsing router arguments");

        for meta in metas {
            if let Some(args) = Self::from_meta(&meta).map_err(syn::Error::from)? {
                debug!("Successfully parsed router arguments");
                return Ok(args);
            }
        }

        Err(syn::Error::new(input.span(), "mode is required"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    #[test]
    fn test_router_args_builder() {
        let args = RouterArgs::builder()
            .mode(RouterMode::Solidity)
            .build()
            .unwrap();
        assert_eq!(args.mode(), RouterMode::Solidity);
    }

    #[test]
    fn test_router_args_builder_missing_mode() {
        let result = RouterArgsBuilder::default().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_router_args_parse() {
        let input = quote! {
            mode = "solidity"
        };

        let args = parse2::<RouterArgs>(input).unwrap();
        assert_eq!(args.mode(), RouterMode::Solidity);
    }

    #[test]
    fn test_router_args_parse_invalid_mode() {
        let input = quote! {
            mode = "invalid"
        };

        let result = parse2::<RouterArgs>(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_router_args_parse_missing_mode() {
        let input = quote! {};

        let result = parse2::<RouterArgs>(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_router_args_case_insensitive() {
        let input = quote! {
            mode = "SOLIDITY"
        };

        let args = parse2::<RouterArgs>(input).unwrap();
        assert_eq!(args.mode(), RouterMode::Solidity);
    }
}

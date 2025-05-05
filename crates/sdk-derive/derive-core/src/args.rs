use crate::{error::RouterError, mode::RouterMode};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
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
    /// Path to the artifacts directory
    pub(crate) artifacts_path: Option<String>,
}

impl RouterArgs {
    /// Creates a new builder for RouterArgs
    pub fn builder() -> RouterArgsBuilder {
        RouterArgsBuilder::default()
    }

    /// Creates a new RouterArgs instance directly
    pub fn new(mode: RouterMode) -> Self {
        Self {
            mode,
            artifacts_path: None,
        }
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
                        artifacts_path: None,
                    }));
                }
            } else if m.path.is_ident("artifacts") {
                debug!("Found artifacts attribute");
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(lit_str),
                    ..
                }) = &m.value
                {
                    return Ok(Some(RouterArgs {
                        mode: RouterMode::Solidity, /* Default mode, will be overridden if mode
                                                     * is also specified */
                        artifacts_path: Some(lit_str.value()),
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
    artifacts_path: Option<String>,
}

impl RouterArgsBuilder {
    /// Sets the routing mode
    pub fn mode(mut self, mode: RouterMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Sets the artifacts path
    pub fn artifacts_path(mut self, path: String) -> Self {
        self.artifacts_path = Some(path);
        self
    }

    /// Builds the RouterArgs instance
    pub fn build(self) -> Result<RouterArgs> {
        let mode = self.mode.ok_or_else(|| {
            warn!("Attempted to build RouterArgs without mode");
            RouterError::InvalidMode("Mode must be specified".to_string())
        })?;

        Ok(RouterArgs {
            mode,
            artifacts_path: self.artifacts_path,
        })
    }
}

impl Parse for RouterArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        debug!("Parsing router arguments");

        let mut mode = None;
        let mut artifacts_path = None;

        for meta in metas {
            if let Some(args) = Self::from_meta(&meta)? {
                if args.mode != RouterMode::Solidity || mode.is_none() {
                    mode = Some(args.mode);
                }
                if args.artifacts_path.is_some() {
                    artifacts_path = args.artifacts_path;
                }
            }
        }

        if let Some(mode) = mode {
            debug!("Successfully parsed router arguments");
            Ok(RouterArgs {
                mode,
                artifacts_path,
            })
        } else {
            Err(syn::Error::new(input.span(), "mode is required"))
        }
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

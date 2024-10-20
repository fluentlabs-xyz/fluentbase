mod fluent;
mod solidity;
mod utils;

use fluent::derive_codec_router;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::abort;
use quote::quote;
use solidity::derive_solidity_router;
use std::str::FromStr;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
    Expr,
    ExprLit,
    ItemFn,
    ItemImpl,
    ItemTrait,
    Lit,
    Meta,
    Token,
};

#[derive(Debug, PartialEq)]
enum RouterMode {
    Solidity,
    Codec,
}

impl FromStr for RouterMode {
    type Err = syn::Error;

    fn from_str(input: &str) -> Result<RouterMode, Self::Err> {
        match input {
            "solidity" => Ok(RouterMode::Solidity),
            "codec" => Ok(RouterMode::Codec),
            _ => Err(syn::Error::new_spanned(
                input,
                "Expected 'solidity' or 'codec'",
            )),
        }
    }
}

#[derive(Debug)]
struct RouterArgs {
    mode: RouterMode,
}

impl Parse for RouterArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mode = None;

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        for meta in metas {
            if let Meta::NameValue(m) = meta {
                if m.path.is_ident("mode") {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }) = &m.value
                    {
                        mode = Some(lit_str.value().parse::<RouterMode>().map_err(|_| {
                            syn::Error::new_spanned(&m.value, "Expected 'solidity' or 'codec'")
                        })?);
                    } else {
                        return Err(syn::Error::new_spanned(&m.value, "Expected a string value"));
                    }
                }
            }
        }

        let mode = mode.ok_or_else(|| syn::Error::new(input.span(), "mode is required"))?;

        Ok(Self { mode })
    }
}

pub fn router_core(attr: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = syn::parse2::<RouterArgs>(attr.clone());

    let mode = match args {
        Ok(args) => args.mode,
        Err(err) => {
            abort!(
                attr.span(),
                format!("Failed to parse router arguments: {err}")
            );
        }
    };

    let router = match mode {
        RouterMode::Solidity => derive_solidity_router(item),
        RouterMode::Codec => derive_codec_router(item),
    }?;

    Ok(router)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_router_core_solidity() {
        let attr = quote! { mode = "solidity" };
        let item = quote! {
            impl RouterAPI for TestROUTER {
                #[function_id("greeting(bytes,address)")]
                fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
                    message
                }
            }
        };

        let result = router_core(attr, item);
        assert!(
            result.is_ok(),
            "router_core должен успешно обработать входные данные"
        );
        println!(">>>>{:+#?}", result.unwrap().to_string());
    }

    #[test]
    fn test_router_core_codec() {
        let attr = quote! { mode = "codec" };
        let item = quote! {
            impl RouterAPI for TestROUTER {
                fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
                    message
                }
            }
        };

        let result = router_core(attr, item);
        assert!(
            result.is_ok(),
            "router_core должен успешно обработать входные данные"
        );
    }

    #[test]
    fn test_router_core_invalid_mode() {
        let attr = quote! { mode = "invalid" };
        let item = quote! {
            impl RouterAPI for TestROUTER {
                fn greeting(&self, message: Bytes, caller: Address) -> Bytes {
                    message
                }
            }
        };

        let result = router_core(attr, item);
        assert!(
            result.is_err(),
            "router_core должен вернуть ошибку для недопустимого режима"
        );
    }
}

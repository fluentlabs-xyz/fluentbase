use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    self,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Expr,
    ExprLit,
    FnArg,
    Ident,
    ImplItem,
    ItemImpl,
    Lit,
    Meta,
    Token,
    Type,
    TypePath,
    Visibility,
};

#[proc_macro]
pub fn derive_keccak256_id(token: TokenStream) -> TokenStream {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(token.to_string());
    let mut dst = [0u8; 4];
    dst.copy_from_slice(hash.finalize().as_slice()[0..4].as_ref());
    let method_id: u32 = u32::from_be_bytes(dst);
    TokenStream::from(quote! {
        #method_id
    })
}

#[derive(Debug)]
struct SolidityRouterInput {
    pub with_main: bool,
}

impl Parse for SolidityRouterInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut with_main = false;

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        for meta in metas {
            if let Meta::NameValue(m) = meta {
                if m.path.is_ident("with_main") {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Bool(lit_bool),
                        ..
                    }) = &m.value
                    {
                        with_main = lit_bool.value();
                    } else {
                        return Err(syn::Error::new_spanned(
                            &m.value,
                            "Expected a boolean value",
                        ));
                    }
                }
            }
        }

        Ok(Self { with_main })
    }
}

#[proc_macro_attribute]
pub fn derive_solidity_router(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(attr as SolidityRouterInput);
    let ast: ItemImpl = parse_macro_input!(item as ItemImpl);

    let struct_name = &ast.self_ty;

    let router_match_arms: Vec<_> = ast
        .items
        .iter()
        .filter_map(expand_router_match_arm)
        .collect();

    let methods: Vec<_> = ast
        .items
        .iter()
        .filter(|item| matches!(item, ImplItem::Fn(_)))
        .collect();

    let expanded = expand_solidity_router(struct_name, router_match_arms, methods, input.with_main);

    TokenStream::from(expanded)
}

fn expand_solidity_router(
    struct_name: &Box<Type>,
    router_match_arms: Vec<proc_macro2::TokenStream>,
    methods: Vec<&ImplItem>,
    with_main: bool,
) -> proc_macro2::TokenStream {
    let router = expand_router(struct_name, router_match_arms, methods.clone());
    let sol_signatures = expand_sol(methods);

    let main_fn = if with_main {
        expand_main_fn(struct_name)
    } else {
        quote! {}
    };

    quote! {
        #router
        #sol_signatures
        #main_fn
    }
}

fn expand_router(
    struct_name: &Box<Type>,
    router_match_arms: Vec<proc_macro2::TokenStream>,
    methods: Vec<&ImplItem>,
) -> proc_macro2::TokenStream {
    quote! {

        impl #struct_name {
            #(#methods)*

            pub fn route(&self, input: &alloc::Vec<u8>) -> alloc::Vec<u8> {
                if input.len() < 4 {
                    panic!("input too short, cannot extract selector");
                }
                let mut selector: [u8; 4] = [0; 4];
                selector.copy_from_slice(&input[0..4]);
                match selector {
                    #(#router_match_arms),*,
                    _ => panic!("unknown method"),
                }
            }
        }
    }
}

fn expand_main_fn(struct_name: &Box<Type>) -> proc_macro2::TokenStream {
    quote! {
        fn main() {
            // Create a default execution context
            let ctx = ExecutionContext::default();
            // Get the contract input
            let input = ctx.contract_input().clone();
            let mut contract = #struct_name(&mut ctx);

            let output = contract.route(&input);
            LowLevelSDK::sys_write(&output);
        }
    }
}

fn expand_router_match_arm(item: &ImplItem) -> Option<proc_macro2::TokenStream> {
    if let ImplItem::Fn(func) = item {
        if let Visibility::Public(_) = func.vis {
            let method_name = &func.sig.ident;
            let method_name_call = sol_call_fn_name(method_name);
            let selector_name = quote! { #method_name_call::SELECTOR };
            let abi_decode = quote! { #method_name_call::abi_decode };

            let param_names: Vec<_> = func
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                            Some(&pat_ident.ident)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            let args_expr = expand_args_expr(&abi_decode, &param_names);

            return Some(quote! {
                #selector_name => {
                    #args_expr
                    self.#method_name(#(#param_names),*).abi_encode()
                }
            });
        }
    }
    None
}

fn expand_args_expr(
    abi_decode: &proc_macro2::TokenStream,
    param_names: &[&Ident],
) -> proc_macro2::TokenStream {
    if param_names.len() == 1 {
        let param_name = &param_names[0];
        quote! {
            let #param_name = match #abi_decode(&input, true) {
                Ok(decoded) => decoded.#param_name,
                Err(e) => {
                    panic!("Failed to decode input {:?} {:?}", stringify!(#param_name), e);
                }
            };
        }
    } else {
        let fields: Vec<proc_macro2::TokenStream> = param_names
            .iter()
            .map(|param_name| quote! { decoded.#param_name })
            .collect();
        quote! {
            let (#(#param_names),*) = match #abi_decode(&input, true) {
                Ok(decoded) => (#(#fields),*),
                Err(e) => {
                    panic!("Failed to decode input {:?}", e);
                }
            };
        }
    }
}

fn expand_sol(methods: Vec<&ImplItem>) -> proc_macro2::TokenStream {
    let sol_functions: Vec<_> = methods
        .iter()
        .filter_map(|item| {
            if let ImplItem::Fn(func) = item {
                if let Visibility::Public(_) = func.vis {
                    let method_name = &func.sig.ident;
                    let sol_method_name = rust_name_to_sol(method_name);

                    // Collect input parameter types and names
                    let inputs = parse_function_inputs(&func.sig.inputs);

                    // Collect output parameter type
                    let output = if let syn::ReturnType::Type(_, ty) = &func.sig.output {
                        rust_type_to_sol(ty)
                    } else {
                        quote! { void }
                    };

                    // Generate function signature in Solidity syntax
                    Some(quote! {
                        function #sol_method_name(#(#inputs),*) external view returns (#output);
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    let sol_block = quote! {
        sol! {
            #(#sol_functions)*
        }
    };

    sol_block
}

fn parse_function_inputs(inputs: &Punctuated<FnArg, Token![,]>) -> Vec<proc_macro2::TokenStream> {
    inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    let param_name = &pat_ident.ident;
                    let sol_type = rust_type_to_sol(&*pat_type.ty);
                    Some(quote! { #sol_type #param_name })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

fn sol_call_fn_name(method_name: &Ident) -> Ident {
    let method_name_sol = rust_name_to_sol(method_name);
    Ident::new(&(method_name_sol.to_string() + "Call"), method_name.span())
}

fn rust_name_to_sol(ident: &Ident) -> Ident {
    let span = ident.span();
    let camel_name = ident.to_string().to_case(Case::Camel);
    Ident::new(&camel_name, span)
}

fn rust_type_to_sol(ty: &Type) -> proc_macro2::TokenStream {
    match ty {
        Type::Array(ty) => convert_array_type(ty),
        Type::Paren(ty) => convert_paren_type(ty),
        Type::Slice(ty) => convert_slice_type(ty),
        Type::Tuple(ty) => convert_tuple_type(ty),
        Type::Path(type_path) => convert_path_type(type_path),
        _ => panic!("Unsupported type: {:?}", ty),
    }
}

fn convert_array_type(ty: &syn::TypeArray) -> proc_macro2::TokenStream {
    if let Expr::Lit(expr_lit) = &ty.len {
        if let Lit::Int(lit_int) = &expr_lit.lit {
            let len = lit_int.base10_digits();
            let len_token: proc_macro2::TokenStream = len.parse().expect("Invalid token");
            let elem_type = rust_type_to_sol(&ty.elem);
            quote! { #elem_type[#len_token] }
        } else {
            panic!("Unsupported array length literal")
        }
    } else {
        panic!("Unsupported array length expression");
    }
}

fn convert_paren_type(ty: &syn::TypeParen) -> proc_macro2::TokenStream {
    rust_type_to_sol(&ty.elem)
}

fn convert_slice_type(ty: &syn::TypeSlice) -> proc_macro2::TokenStream {
    let elem_type = rust_type_to_sol(&ty.elem);
    quote! { #elem_type[] }
}

fn convert_tuple_type(ty: &syn::TypeTuple) -> proc_macro2::TokenStream {
    let elems = ty.elems.iter().map(|elem| rust_type_to_sol(elem));
    quote! { (#(#elems),*)}
}

fn convert_path_type(type_path: &TypePath) -> proc_macro2::TokenStream {
    let ident = &type_path.path.segments.last().unwrap().ident;
    match ident.to_string().as_str() {
        "String" => quote! { string },
        "bool" => quote! { bool },
        "u8" => quote! { uint8 },
        "u16" => quote! { uint16 },
        "u32" => quote! { uint32 },
        "u64" => quote! { uint64 },
        "u128" => quote! { uint128 },
        "u256" | "uint" => quote! { uint256 },
        "i8" => quote! { int8 },
        "i16" => quote! { int16 },
        "i32" => quote! { int32 },
        "i64" => quote! { int64 },
        "i128" => quote! { int128 },
        "i256" | "int" => quote! { int256 },
        "Address" => quote! { address },
        "Vec" => {
            if let syn::PathArguments::AngleBracketed(args) =
                &type_path.path.segments.last().unwrap().arguments
            {
                if let Some(syn::GenericArgument::Type(arg_ty)) = args.args.first() {
                    let elem_type = rust_type_to_sol(arg_ty);
                    quote! { #elem_type[] }
                } else {
                    panic!("Unsupported vector element type")
                }
            } else {
                panic!("Unsupported vector arguments")
            }
        }
        ident_str if ident_str.starts_with("bytes") => {
            if ident_str == "bytes" {
                quote! { bytes }
            } else if ident_str.len() > 5 && ident_str[5..].parse::<usize>().is_ok() {
                let bytes_len: usize = ident_str[5..].parse().unwrap();
                let bytes_ident = Ident::new(&format!("bytes{}", bytes_len), ident.span());
                quote! { #bytes_ident }
            } else {
                panic!("Unsupported bytes type: {}", ident_str)
            }
        }
        _ => panic!("Unsupported type: {}", ident),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_quote, Ident, ImplItem, TypeArray, TypeParen, TypePath, TypeSlice, TypeTuple};

    #[test]
    fn test_rust_name_to_sol() {
        let ident = Ident::new("test_function", proc_macro2::Span::call_site());
        let sol_ident = rust_name_to_sol(&ident);
        assert_eq!(sol_ident.to_string(), "testFunction");
    }

    #[test]
    fn test_get_method_call() {
        let method_name = Ident::new("test_function", proc_macro2::Span::call_site());
        let method_call_ident = sol_call_fn_name(&method_name);
        assert_eq!(method_call_ident.to_string(), "testFunctionCall");
    }

    #[test]
    fn test_expand_match_arm_single_param() {
        let item: ImplItem = parse_quote! {
            pub fn is_checkmate(&self, board: String) -> bool {
                true
            }
        };

        let expected = quote! {
            isCheckmateCall::SELECTOR => {
                let board = match isCheckmateCall::abi_decode(&input, true) {
                    Ok(decoded) => decoded.board,
                    Err(e) => {
                        panic!("Failed to decode input {:?} {:?}", stringify!(board), e);
                    }
                };
                self.is_checkmate(board).abi_encode()
            }
        };

        if let Some(actual) = expand_router_match_arm(&item) {
            assert_eq!(actual.to_string(), expected.to_string());
        } else {
            panic!("Failed to expand match arm");
        }
    }

    #[test]
    fn test_expand_match_arm_multiple_params() {
        let item: ImplItem = parse_quote! {
            pub fn is_checkmate(&self, board: String, mv: String) -> bool {
                true
            }
        };

        let expected = quote! {
            isCheckmateCall::SELECTOR => {
                let (board, mv) = match isCheckmateCall::abi_decode(&input, true) {
                    Ok(decoded) => (decoded.board, decoded.mv),
                    Err(e) => {
                        panic!("Failed to decode input {:?}", e);
                    }
                };
                self.is_checkmate(board, mv).abi_encode()
            }
        };

        if let Some(actual) = expand_router_match_arm(&item) {
            assert_eq!(actual.to_string(), expected.to_string());
        } else {
            panic!("Failed to expand match arm");
        }
    }

    #[test]
    fn test_expand_args_expr_single_param() {
        let abi_decode: proc_macro2::TokenStream = quote! { isCheckmateCall::abi_decode };
        let param_board = Ident::new("board", proc_macro2::Span::call_site());
        let param_names = vec![&param_board];

        let args_expr = expand_args_expr(&abi_decode, &param_names);
        let expected = quote! {
            let board = match isCheckmateCall::abi_decode(&input, true) {
                Ok(decoded) => decoded.board,
                Err(e) => {
                    panic!("Failed to decode input {:?} {:?}", stringify!(board), e);
                }
            };
        };

        assert_eq!(args_expr.to_string(), expected.to_string());
    }

    #[test]
    fn test_expand_args_expr_multiple_params() {
        let abi_decode: proc_macro2::TokenStream = quote! { isCheckmateCall::abi_decode };
        let param_board = Ident::new("board", proc_macro2::Span::call_site());
        let param_mv = Ident::new("mv", proc_macro2::Span::call_site());
        let param_names = vec![&param_board, &param_mv];

        let args_expr = expand_args_expr(&abi_decode, &param_names);
        let expected = quote! {
            let (board, mv) = match isCheckmateCall::abi_decode(&input, true) {
                Ok(decoded) => (decoded.board, decoded.mv),
                Err(e) => {
                    panic!("Failed to decode input {:?}", e);
                }
            };
        };

        assert_eq!(args_expr.to_string(), expected.to_string());
    }

    #[test]
    fn test_convert_array_type() {
        let ty: TypeArray = parse_quote!([u8; 32]);
        let result = convert_array_type(&ty);
        assert_eq!(result.to_string(), "uint8 [32]");
    }

    #[test]
    fn test_convert_paren_type() {
        let ty: TypeParen = parse_quote!((u8));
        let result = convert_paren_type(&ty);
        assert_eq!(result.to_string(), "uint8");
    }

    #[test]
    fn test_convert_slice_type() {
        let ty: TypeSlice = parse_quote!([u8]);
        let result = convert_slice_type(&ty);
        assert_eq!(result.to_string(), "uint8 []");
    }

    #[test]
    fn test_convert_tuple_type() {
        let ty: TypeTuple = parse_quote!((u8, u16));
        let result = convert_tuple_type(&ty);
        assert_eq!(result.to_string(), "(uint8 , uint16)");
    }

    #[test]
    fn test_convert_path_type_string() {
        let ty: TypePath = parse_quote!(String);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "string");
    }

    #[test]
    fn test_convert_path_type_bool() {
        let ty: TypePath = parse_quote!(bool);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "bool");
    }

    #[test]
    fn test_convert_path_type_uint() {
        let ty: TypePath = parse_quote!(u256);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "uint256");
    }

    #[test]
    fn test_convert_path_type_vec() {
        let ty: TypePath = parse_quote!(Vec<u8>);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "uint8 []");
    }

    #[test]
    fn test_convert_path_type_bytes() {
        let ty: TypePath = parse_quote!(bytes);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "bytes");
    }

    #[test]
    fn test_convert_path_type_bytes_fixed() {
        let ty: TypePath = parse_quote!(bytes32);
        let result = convert_path_type(&ty);
        assert_eq!(result.to_string(), "bytes32");
    }
}

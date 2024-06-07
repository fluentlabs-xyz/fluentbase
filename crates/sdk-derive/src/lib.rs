use proc_macro::TokenStream;

use convert_case::{Case, Casing};
use quote::quote;
use syn::{
    self,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Expr, ExprLit, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, Lit, LitStr, Meta, Token, Type,
    TypePath, Visibility,
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

// Fake implementation of the attribute to avoid compiler and linter complaints
#[proc_macro_attribute]
pub fn sol_signature(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn derive_solidity_router(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(attr as SolidityRouterInput);
    let ast: ItemImpl = parse_macro_input!(item as ItemImpl);

    let struct_name = &ast.self_ty;

    // Get all public methods from the impl block
    let methods_to_route = get_methods_to_route(&ast);
    // Generate Solidity function signatures or use provided ones from #[sol_signature]
    let sol_signatures = get_sol_signatures(&methods_to_route);
    // Derive route method that dispatches Solidity function calls
    let router = derive_route_method(struct_name, &methods_to_route);
    // Generate main function that is used in the contract entry point
    let main_fn = if input.with_main {
        derive_main_fn(struct_name)
    } else {
        quote! {}
    };

    let expanded = quote! {
        #router
        #sol_signatures
        #main_fn
    };

    TokenStream::from(expanded)
}

fn get_methods_to_route(ast: &ItemImpl) -> Vec<&syn::ImplItemFn> {
    let all_methods: Vec<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            if let ImplItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect();

    let public_methods: Vec<&ImplItemFn> = all_methods
        .into_iter()
        .filter(|func| matches!(func.vis, Visibility::Public(_)))
        .collect();

    public_methods
}

fn get_sol_signatures(methods: &[&ImplItemFn]) -> proc_macro2::TokenStream {
    let mut signatures: Vec<proc_macro2::TokenStream> = vec![];
    for func in methods {
        let sig: Option<LitStr> = func.attrs.iter().find_map(|attr| {
            if attr.path().is_ident("sol_signature") {
                attr.parse_args().ok()
            } else {
                None
            }
        });

        if let Some(fn_signature) = sig {
            let fn_signature = fn_signature.value() + "; ";
            let fn_signature = syn::parse_str::<proc_macro2::TokenStream>(&fn_signature)
                .expect("Failed to parse signature");
            signatures.push(fn_signature);
        } else {
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
            signatures.push(quote! {
                function #sol_method_name(#(#inputs),*) external view returns (#output);
            });
        }
    }
    quote! {
        sol! {
            #(#signatures)*
        }
    }
}

fn derive_route_method(
    struct_name: &Box<Type>,
    methods: &Vec<&ImplItemFn>,
) -> proc_macro2::TokenStream {
    let selectors: Vec<proc_macro2::TokenStream> = methods
        .iter()
        .filter_map(|method| {
            let selector = derive_route_selector_arm(method);
            Some(selector)
        })
        .collect();

    quote! {
        impl #struct_name {
            #(#methods)*

            pub fn route(&self, input: &[u8], output: &mut [u8]) {
                if input.len() < 4 {
                    panic!("input too short, cannot extract selector");
                }
                let mut selector: [u8; 4] = [0; 4];
                selector.copy_from_slice(&input[0..4]);
                match selector {
                    #(#selectors),*,
                    _ => panic!("unknown method"),
                }
            }
        }
    }
}

fn derive_route_selector_arm(func: &ImplItemFn) -> proc_macro2::TokenStream {
    let method_name = &func.sig.ident;
    let method_name_call = sol_call_fn_name(method_name);
    let selector_name = quote! { #method_name_call::SELECTOR };
    let abi_decode = quote! { #method_name_call::abi_decode };

    let args: Vec<_> = func
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

    let args_expr = derive_route_selector_args(&args, &abi_decode);

    quote! {
        #selector_name => {
            #args_expr
            let encoded_output = self.#method_name(#(#args),*).abi_encode();
            if encoded_output.len() > output.len() {
                panic!("output buffer too small");
            };
            output[..encoded_output.len()].copy_from_slice(&encoded_output);
        }
    }
}

fn derive_route_selector_args(
    args: &[&Ident],
    abi_decode_fn: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if args.len() == 1 {
        let arg = args[0];
        quote! {
            let #arg = match #abi_decode_fn(&input, true) {
                Ok(decoded) => decoded.#arg,
                Err(e) => {
                    panic!("Failed to decode input {:?} {:?}", stringify!(#arg), e);
                }
            };
        }
    } else {
        let fields: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| quote! { decoded.#arg }).collect();
        quote! {
            let (#(#args),*) = match #abi_decode_fn(&input, true) {
                Ok(decoded) => (#(#fields),*),
                Err(e) => {
                    panic!("Failed to decode input {:?}", e);
                }
            };
        }
    }
}

fn derive_main_fn(struct_name: &Box<Type>) -> proc_macro2::TokenStream {
    quote! {
        #[cfg(not(feature = "std"))]
        #[no_mangle]
        #[cfg(target_arch = "wasm32")]
        pub extern "C" fn main() {
            // Create a default execution context
            let ctx = ExecutionContext::default();
            // Get the contract input
            let input = ctx.contract_input().to_vec();
            let mut contract = #struct_name {};

            let output = contract.route(&input);
            LowLevelSDK::sys_write(&output);
        }
    }
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
        Type::Reference(type_ref) => rust_type_to_sol(&type_ref.elem),
        _ => panic!("Unsupported type: {}", ty.to_token_stream().to_string()),
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
        "String" | "str" => quote! { string },
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
    use syn::{parse_quote, Ident, TypeArray, TypeParen, TypePath, TypeSlice, TypeTuple};

    use super::*;

    #[test]
    fn test_derive_route_selector_arm_single_param() {
        let item: ImplItemFn = parse_quote! {
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

        let actual = derive_route_selector_arm(&item);
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_derive_route_selector_arm_multiple_params() {
        let item: ImplItemFn = parse_quote! {
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

        let actual = derive_route_selector_arm(&item);
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_get_sol_signatures() {
        let item_impl: ItemImpl = parse_quote! {
            impl ExampleStruct {
                #[sol_signature("function greeting() external view returns ()")]
                pub fn greeting(&self, msg: String) -> String {
                    msg
                }

                pub fn regular_fn(&self, msg: String) -> String {
                    msg
                }
                pub fn greeting_msg(&self, msg: String) -> String {
                    msg
                }
            }
        };

        let methods = get_methods_to_route(&item_impl);
        let sol_signatures = get_sol_signatures(&methods);

        let expected = quote! {
            sol! {
                function greeting() external view returns ();
                function regularFn(string msg) external view returns (string);
                function greetingMsg(string msg) external view returns (string);
            }
        };

        assert_eq!(sol_signatures.to_string(), expected.to_string());
    }

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
        let ty: TypePath = parse_quote!(str);
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

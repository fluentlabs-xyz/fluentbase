use convert_case::{Case, Casing};
use proc_macro2::Ident;
use quote::{quote, ToTokens};
use syn::{
    punctuated::Punctuated,
    Attribute,
    Expr,
    FnArg,
    ImplItem,
    ImplItemFn,
    ItemImpl,
    Lit,
    LitStr,
    ReturnType,
    Signature,
    Token,
    TraitItemFn,
    Type,
    TypePath,
    Visibility,
};

pub fn get_all_methods(ast: &ItemImpl) -> Vec<&ImplItemFn> {
    ast.items
        .iter()
        .filter_map(|item| {
            if let ImplItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect()
}

pub fn get_public_methods(ast: &ItemImpl) -> Vec<&ImplItemFn> {
    get_all_methods(ast)
        .into_iter()
        .filter(|func| matches!(func.vis, Visibility::Public(_)))
        .collect()
}

pub fn calculate_keccak256_bytes(signature: &str) -> [u8; 4] {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(signature);
    let mut dst = [0u8; 4];
    dst.copy_from_slice(hash.finalize().as_slice()[0..4].as_ref());
    dst
}

pub fn calculate_keccak256_id(signature: &str) -> u32 {
    u32::from_be_bytes(calculate_keccak256_bytes(signature.trim_matches('"')))
}

pub fn parse_function_inputs(
    inputs: &Punctuated<FnArg, Token![,]>,
) -> Vec<proc_macro2::TokenStream> {
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

pub fn parse_function_input_types(
    inputs: &Punctuated<FnArg, Token![,]>,
) -> Vec<proc_macro2::TokenStream> {
    inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                let sol_type = rust_type_to_sol(&*pat_type.ty);
                Some(quote! { #sol_type })
            } else {
                None
            }
        })
        .collect()
}

pub fn sol_call_fn_name(method_name: &Ident) -> Ident {
    let method_name_sol = rust_name_to_sol(method_name);
    Ident::new(&(method_name_sol.to_string() + "Call"), method_name.span())
}

pub fn rust_name_to_sol(ident: &Ident) -> Ident {
    let span = ident.span();
    let camel_name = ident.to_string().to_case(Case::Camel);
    Ident::new(&camel_name, span)
}

pub fn rust_type_to_sol(ty: &Type) -> proc_macro2::TokenStream {
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
        "U256" => quote! { uint256 },
        "i8" => quote! { int8 },
        "i16" => quote! { int16 },
        "i32" => quote! { int32 },
        "i64" => quote! { int64 },
        "i128" => quote! { int128 },
        "i256" | "int" => quote! { int256 },
        "Address" => quote! { address },
        "Bytes" => quote! { bytes },
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

pub(crate) trait GetSignature {
    fn attrs(&self) -> &Vec<Attribute>;
    fn sig(&self) -> &Signature;
}

impl GetSignature for ImplItemFn {
    fn attrs(&self) -> &Vec<Attribute> {
        &self.attrs
    }

    fn sig(&self) -> &Signature {
        &self.sig
    }
}

impl GetSignature for TraitItemFn {
    fn attrs(&self) -> &Vec<Attribute> {
        &self.attrs
    }

    fn sig(&self) -> &Signature {
        &self.sig
    }
}

pub(crate) fn get_raw_signature<S: GetSignature>(func: &S) -> proc_macro2::TokenStream {
    let sig: Option<LitStr> = func.attrs().iter().find_map(|attr| {
        if attr.path().is_ident("signature") {
            attr.parse_args().ok()
        } else {
            None
        }
    });
    if let Some(fn_signature) = sig {
        quote! {
            #fn_signature
        }
    } else {
        let method_name = &func.sig().ident;
        let sol_method_name = rust_name_to_sol(method_name);
        let inputs = parse_function_input_types(&func.sig().inputs);
        let inputs = inputs
            .into_iter()
            .map(|i| i.to_string())
            .collect::<Vec<String>>()
            .join(",");
        quote! {
            #sol_method_name #inputs
        }
    }
}

pub(crate) fn get_signature<S: GetSignature>(func: &S) -> proc_macro2::TokenStream {
    let sig: Option<LitStr> = func.attrs().iter().find_map(|attr| {
        if attr.path().is_ident("signature") {
            attr.parse_args().ok()
        } else {
            None
        }
    });
    if let Some(fn_signature) = sig {
        let signature_value = fn_signature.value();
        let full_signature = if signature_value.starts_with("function ") {
            signature_value + "; "
        } else {
            let method_name = &func.sig().ident;
            let sol_method_name = rust_name_to_sol(method_name);

            let inputs = parse_function_inputs(&func.sig().inputs);
            let inputs = inputs
                .into_iter()
                .map(|i| i.to_string())
                .collect::<Vec<String>>()
                .join(", ");
            if let ReturnType::Type(_, ty) = &func.sig().output {
                format!(
                    "function {}({}) external returns ({});",
                    sol_method_name,
                    inputs,
                    rust_type_to_sol(ty).to_string()
                )
            } else {
                format!("function {}({}) external;", sol_method_name, inputs,)
            }
        };
        syn::parse_str::<proc_macro2::TokenStream>(&full_signature)
            .expect("failed to parse signature")
    } else {
        let method_name = &func.sig().ident;
        let sol_method_name = rust_name_to_sol(method_name);

        let inputs = parse_function_inputs(&func.sig().inputs);
        let output = if let syn::ReturnType::Type(_, ty) = &func.sig().output {
            let output = rust_type_to_sol(ty);
            quote! {
                function #sol_method_name(#(#inputs),*) external returns (#output);
            }
        } else {
            quote! {
                function #sol_method_name(#(#inputs),*) external;
            }
        };
        // Generate function signature in Solidity syntax
        output
    }
}

pub(crate) fn get_signatures<S: GetSignature>(methods: &[&S]) -> proc_macro2::TokenStream {
    let mut signatures: Vec<proc_macro2::TokenStream> = vec![];
    for func in methods {
        signatures.push(get_signature(*func));
    }
    quote! {
        sol! {
            #(#signatures)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_quote, TypeArray, TypeParen, TypeSlice, TypeTuple};

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

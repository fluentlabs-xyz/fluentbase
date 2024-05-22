use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{self, Expr, FnArg, Ident, ImplItem, Type, Visibility};

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

fn rust_type_to_sol(ty: &Type) -> String {
    let mut result = String::new();
    match &ty {
        Type::Array(ty) => {
            match &ty.len {
                Expr::Const(val) => {
                    unreachable!("how to extract int len? {:?}", val.to_token_stream());
                }
                _ => unreachable!("not supported expr: {:?}", ty.len.to_token_stream()),
            }
            // result.extend(&rust_type_to_solidity(&ty.elem));
            // result.extend("[]");
        }
        Type::Paren(ty) => {
            result.extend(rust_type_to_sol(&ty.elem).chars());
        }
        Type::Slice(ty) => {
            result.extend(rust_type_to_sol(&ty.elem).chars());
            result.extend("[]".chars());
        }
        Type::Tuple(ty) => {
            for ty in ty.elems.iter() {
                result.extend(rust_type_to_sol(&ty).chars());
                result.extend(",".chars());
            }
        }
        _ => unreachable!("not supported type: {:?}", ty.to_token_stream()),
    }
    result
}

fn rust_name_to_sol(ident: &Ident) -> Ident {
    let span = ident.span();
    let camel_name = ident.to_string().to_case(Case::Camel);
    Ident::new(camel_name.as_str(), span)
}

#[proc_macro]
pub fn derive_solidity_router(token: TokenStream) -> TokenStream {
    let ast: syn::ItemImpl = syn::parse(token).unwrap();
    for item in &ast.items {
        match item {
            ImplItem::Fn(func) => {
                match func.vis {
                    Visibility::Public(_) => {}
                    _ => continue,
                }
                let ident = rust_name_to_sol(&func.sig.ident);
                println!("{:?}", ident.to_string());
                for param in func.sig.inputs.iter() {
                    let param = match param {
                        FnArg::Receiver(_) => continue,
                        FnArg::Typed(param) => param,
                    };
                    let ty = rust_type_to_sol(&param.ty);
                    println!("{:?}", ty.to_string());
                }
            }
            _ => continue,
        }
    }
    TokenStream::from(quote! {})
}

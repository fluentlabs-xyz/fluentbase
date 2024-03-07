#![no_std]

extern crate alloc;
extern crate proc_macro;

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::{convert::TryInto, fmt::Debug, hash::Hash};
use crypto_hashes::md2::{Digest, Md2};
use proc_macro::TokenStream;
use syn::{
    punctuated::Punctuated,
    token::PathSep,
    FnArg,
    ForeignItem,
    ItemForeignMod,
    Pat,
    Path,
    PathSegment,
    Type,
};

#[proc_macro]
pub fn derive_helpers_and_structs(tokens: TokenStream) -> TokenStream {
    let foreign_mod_ast = syn::parse::<ItemForeignMod>(tokens.clone()).unwrap();
    assert_eq!("C", foreign_mod_ast.clone().abi.name.unwrap().value());

    let mut use_decls = String::new();
    let mut const_decls = String::new();
    let mut struct_decls = String::new();
    let mut enum_decls = String::new();
    let mut fn_decls = String::new();

    use_decls.push_str(
        r#"
            use fluentbase_codec::{define_codec_struct, BufferDecoder, Encoder};
            use alloc::{vec::Vec, string::{String, ToString}};
        "#,
    );

    let mut struct_ident_prefix_to_const_ident = Vec::<(String, String)>::new();
    for fn_item in &foreign_mod_ast.clone().items {
        match fn_item {
            ForeignItem::Fn(fn_instance) => {
                let mut ident = fn_instance.sig.ident.to_string();
                if ident.starts_with("_") {
                    let mut do_upper = true;
                    ident = ident.trim_start_matches("_").to_string();
                    let const_ident_prefix = ident.to_uppercase();
                    let const_ident = format!("{const_ident_prefix}_METHOD_ID");
                    let mut struct_ident_prefix: String = ident
                        .chars()
                        .map(|mut v| {
                            if v == '_' {
                                do_upper = true;
                            } else if do_upper {
                                v = v.to_ascii_uppercase();
                                do_upper = false;
                            }
                            v
                        })
                        .into_iter()
                        .collect();
                    struct_ident_prefix = struct_ident_prefix.replace("_", "");
                    let struct_ident = format!("{}{}", struct_ident_prefix, "MethodInput");
                    struct_ident_prefix_to_const_ident
                        .push((struct_ident_prefix.clone(), const_ident.clone()));
                    let mut h = Md2::default();
                    h.update(struct_ident.clone());
                    let mut dst = [0u8; 4];
                    dst.copy_from_slice(h.finalize().as_slice()[0..4].as_ref());
                    let mut method_id: u32 = u32::from_be_bytes(dst);
                    const_decls.push_str(
                        format!("pub const {const_ident}: u32 = {method_id};\n").as_str(),
                    );
                    let mut source_field_idents = Vec::<String>::new();
                    let mut field_name_and_type: Vec<(String, String)> = Default::default();
                    for fn_arg in &fn_instance.sig.inputs {
                        match fn_arg {
                            FnArg::Typed(pat_type) => {
                                let field_ident = match pat_type.pat.as_ref() {
                                    Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
                                    _ => {
                                        panic!("unsupported parameter ident type")
                                    }
                                };
                                source_field_idents.push(field_ident.clone());
                                match pat_type.ty.as_ref() {
                                    Type::Path(type_path) => {
                                        assert_eq!(1, type_path.path.segments.len());
                                        let variable_ident =
                                            type_path.path.segments[0].ident.to_string();
                                        if variable_ident == "u32" {
                                            if !(field_ident.ends_with("_len")
                                                && source_field_idents.contains(
                                                    &field_ident.replace("_len", "_offset"),
                                                )
                                                || field_ident.ends_with("_size")
                                                    && source_field_idents.contains(
                                                        &field_ident.replace("_size", "_offset"),
                                                    ))
                                            {
                                                field_name_and_type
                                                    .push((field_ident, variable_ident));
                                            };
                                        } else {
                                            panic!("unsupported var ident '{}'", variable_ident)
                                        }
                                    }
                                    Type::Ptr(type_ptr) => {
                                        assert!(type_ptr.const_token.is_some());
                                        match type_ptr.elem.as_ref() {
                                            Type::Path(v) => {
                                                assert_eq!(1, v.path.segments.len());
                                                let const_pointer_ident =
                                                    v.path.segments[0].ident.to_string();
                                                if const_pointer_ident == "u8" {
                                                    // TODO check field name has 'offset' suffix
                                                    if field_ident.ends_with("32_offset") {
                                                        let field_name = field_ident
                                                            .trim_end_matches("_offset")
                                                            .to_string();
                                                        field_name_and_type.push((
                                                            field_name,
                                                            "[u8; 32]".to_string(),
                                                        ));
                                                    } else if field_ident.ends_with("20_offset") {
                                                        let field_name = field_ident
                                                            .trim_end_matches("_offset")
                                                            .to_string();
                                                        field_name_and_type.push((
                                                            field_name,
                                                            "[u8; 20]".to_string(),
                                                        ));
                                                    } else if field_ident.ends_with("_offset") {
                                                        let field_name = field_ident
                                                            .trim_end_matches("_offset")
                                                            .to_string();
                                                        field_name_and_type.push((
                                                            field_name,
                                                            "Vec<u8>".to_string(),
                                                        ));
                                                    } else {
                                                        panic!("unsupported field name")
                                                    }
                                                    // TODO fetch number from
                                                } else {
                                                    panic!(
                                                        "unsupported const pointer ident '{}'",
                                                        const_pointer_ident
                                                    )
                                                }
                                            }
                                            _ => panic!("unsupported elem type"),
                                        }
                                    }
                                    _ => {
                                        panic!("unsupported parameter type")
                                    }
                                }
                            }
                            _ => {
                                panic!("unsupported fn arg")
                            }
                        }
                    }
                    struct_decls.push_str(
                        r#"
                        define_codec_struct! {
                            pub struct #STRUCT_IDENT# {
                                #FIELD_NAMES_TO_TYPES#
                            }
                        }
                        impl #STRUCT_IDENT# {
                            pub fn new(#FIELD_NAMES_TO_TYPES#) -> Self {
                                #STRUCT_IDENT# {
                                    #FIELD_NAMES#
                                }
                            }
                        }
                        "#
                        .replace("#STRUCT_IDENT#", &struct_ident)
                        .replace(
                            "#FIELD_NAMES_TO_TYPES#",
                            &field_name_and_type
                                .iter()
                                .map(|(field_name, field_type)| {
                                    format!("{}: {}", field_name, field_type)
                                })
                                .collect::<Vec<String>>()
                                .join(", "),
                        )
                        .replace(
                            "#FIELD_NAMES#",
                            &field_name_and_type
                                .iter()
                                .map(|(field_name, _)| field_name.clone())
                                .collect::<Vec<String>>()
                                .join(", "),
                        )
                        .as_str(),
                    );
                } else {
                    panic!("each function name must start with underscore prefix")
                }
            }
            _ => {
                panic!(
                    "unsupported item type inside extern declaration. only fn types are supported"
                )
            }
        }
    }

    enum_decls.push_str(
        r#"
            #[derive(Copy, Clone)]
            pub enum #STRUCT_NAME# {
                // ex.: EvmCreate = EVM_CREATE_METHOD_ID as isize,
                #ENUM_ITEMS_DEFINITION#
            }
            impl TryFrom<u32> for #STRUCT_NAME# {
                type Error = ();
            
                fn try_from(value: u32) -> Result<Self, Self::Error> {
                    match value {
                        // EVM_CREATE_METHOD_ID => Ok(EVMMethodName::EvmCreate),
                        #ENUM_TRY_FROM_IMPL_HANDS#
                        _ => Err(Self::Error::default()),
                    }
                }
            }
            impl Into<u32> for #STRUCT_NAME# {
                fn into(self) -> u32 {
                    self as u32
                }
            }
        "#
        .replace(
            "#ENUM_ITEMS_DEFINITION#",
            &struct_ident_prefix_to_const_ident
                .iter()
                .map(|(struct_ident_prefix, const_ident)| {
                    format!("{} = {} as isize", struct_ident_prefix, const_ident)
                })
                .collect::<Vec<String>>()
                .join(", "),
        )
        .replace("#ENUM_TRY_FROM_IMPL_HANDS#", &{
            let mut res = struct_ident_prefix_to_const_ident
                .iter()
                .map(|(struct_ident_prefix, const_ident)| {
                    format!(
                        "{} => Ok(#STRUCT_NAME#::{})",
                        const_ident, struct_ident_prefix
                    )
                })
                .collect::<Vec<String>>()
                .join(", ");
            res.push_str(", ");
            res
        })
        .replace("#STRUCT_NAME#", "EVMMethodName")
        .as_str(),
    );

    let ast_debug_string = format!("{:#?}", foreign_mod_ast);
    let mut builder = String::new();
    builder.push_str(
        r#"
        __USE_DECLS__
        __CONST_DECLS__
        __ENUM_DECLS__
        __STRUCT_DECLS__
        __FN_DECLS__
    "#,
    );
    if cfg!(feature = "debug") {
        builder.push_str(
            r#"
            pub fn ast_debug_str() -> String { "__AST_STR__".to_string() }
            pub fn use_decls_debug_str() -> String { "__USE_DECLS__".to_string() }
            pub fn const_decls_debug_str() -> String { "__CONST_DECLS__".to_string() }
            pub fn enum_decls_debug_str() -> String { "__ENUM_DECLS__".to_string() }
            pub fn struct_decls_debug_str() -> String { "__STRUCT_DECLS__".to_string() }
            pub fn fn_decls_debug_str() -> String { "__FN_DECLS__".to_string() }
        "#,
        )
    }
    builder
        .replace(
            r#"__AST_STR__"#,
            &ast_debug_string.escape_unicode().to_string(),
        )
        .replace(r#"__USE_DECLS__"#, &use_decls.to_string())
        .replace(r#"__CONST_DECLS__"#, &const_decls.to_string())
        .replace(r#"__ENUM_DECLS__"#, &enum_decls.to_string())
        .replace(r#"__STRUCT_DECLS__"#, &struct_decls.to_string())
        .replace(r#"__FN_DECLS__"#, &fn_decls.to_string())
        .parse()
        .unwrap()
}

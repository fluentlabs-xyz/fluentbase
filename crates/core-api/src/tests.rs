#[cfg(feature = "debug")]
#[test]
fn proc_macro_output_test() {
    use crate::bindings::{
        ast_debug_str,
        const_decls_debug_str,
        enum_decls_debug_str,
        struct_decls_debug_str,
        use_decls_debug_str,
    };

    println!("use_decls_debug_str: '\n{}'", &use_decls_debug_str());
    println!("const_decls_debug_str: '\n{}'", &const_decls_debug_str());
    println!("enum_decls_debug_str: '\n{}'", &enum_decls_debug_str());
    println!("struct_decls_debug_str: '\n{}'", &struct_decls_debug_str());
    println!("ast_debug_str: {}", &ast_debug_str());
}

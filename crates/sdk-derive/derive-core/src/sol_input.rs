use crate::abi::types::{convert_solidity_type, sol_to_rust};
use alloy_sol_macro_input::{SolInput, SolInputKind};
use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::path::Path;
use syn::{Ident, LitStr, Type};
use syn_solidity::{
    visit::{visit_file, Visit},
    File, Item, ItemFunction, ItemStruct, Mutability, Spanned, VariableDeclaration,
};

/// A visitor that collects Solidity functions and structs
#[derive(Default)]
struct Collector<'a> {
    functions: Vec<&'a ItemFunction>,
    structs: Vec<&'a ItemStruct>,
}

impl<'a> Visit<'a> for Collector<'a> {
    fn visit_item_function(&mut self, func: &'a ItemFunction) {
        self.functions.push(func);
    }
    fn visit_item_struct(&mut self, s: &'a ItemStruct) {
        self.structs.push(s);
    }
}

/// Converts Solidity input to Rust trait token stream
///
/// # Arguments
///
/// * `input` - The Solidity input to convert
///
/// # Returns
///
/// A TokenStream representing the generated Rust trait
pub fn to_rust_trait(input: SolInput) -> syn::Result<TokenStream> {
    let source_dependency = source_file_dependency(input.path.as_deref());
    let (structs, trait_name, trait_fns) = convert_sol_to_rust(input)?;

    // Generate the final output for trait
    Ok(quote! {
        #source_dependency
        #(#structs)*
        pub trait #trait_name {
            #(#trait_fns)*
        }
    })
}

/// Converts Solidity input to Rust client trait with solidity mode
///
/// # Arguments
///
/// * `input` - The Solidity input to convert
///
/// # Returns
///
/// A TokenStream representing the generated Rust client trait
pub fn to_sol_client(input: SolInput) -> syn::Result<TokenStream> {
    let source_dependency = source_file_dependency(input.path.as_deref());
    let (structs, trait_name, trait_fns) = convert_sol_to_rust(input)?;

    // Generate the final output for client trait with attribute
    Ok(quote! {
        #source_dependency
        #(#structs)*
        #[client(mode = "solidity")]
        pub trait #trait_name {
            #(#trait_fns)*
        }
    })
}

/// Declares the `.sol` file a path-form macro read as a build input
///
/// The macro reads the file itself, so without this rustc never learns the file
/// was consumed and Cargo keeps the dependent crate fresh after the interface
/// changes, leaving stale methods and selectors in the build. A discarded
/// `include_str!` puts the path into rustc's dep-info without emitting data.
///
/// # Arguments
///
/// * `path` - The resolved path of the Solidity file, or `None` for inline input
///
/// # Returns
///
/// A TokenStream declaring the dependency, empty when there is no file to track
fn source_file_dependency(path: Option<&Path>) -> TokenStream {
    // A non-UTF-8 path cannot be spelled as a string literal, and `include_str!`
    // takes nothing else, so such a path stays untracked
    let Some(path) = path.and_then(Path::to_str) else {
        return quote! {};
    };
    let path = LitStr::new(path, Span::call_site());

    quote! {
        const _: &::core::primitive::str = ::core::include_str!(#path);
    }
}

/// Converts Solidity input to Rust code components
///
/// # Arguments
///
/// * `input` - The Solidity input to convert
///
/// # Returns
///
/// A tuple of (structs, trait_name, trait_methods) to be assembled
fn convert_sol_to_rust(
    input: SolInput,
) -> syn::Result<(Vec<TokenStream>, Ident, Vec<TokenStream>)> {
    // Get the Solidity file from the input
    let file = match input.kind {
        SolInputKind::Sol(sol_file) => sol_file,
        SolInputKind::Type(_) => {
            return Err(syn::Error::new(
                Span::call_site(),
                "Expected Solidity interface or contract, not type",
            ));
        }
        SolInputKind::Json(_, _) => {
            return Err(syn::Error::new(
                Span::call_site(),
                "JSON ABI not supported in this macro",
            ));
        }
    };

    let mut visitor = Collector::default();
    visit_file(&mut visitor, &file);

    let trait_name = derive_trait_name(&file)?;

    let structs = visitor
        .structs
        .iter()
        .map(|s| sol_struct_to_rust_tokens(s))
        .collect::<syn::Result<Vec<_>>>()?;

    // Every function must convert: dropping one would silently shrink the interface surface.
    let mut trait_fns = Vec::new();
    let mut errors = Vec::new();
    for func in &visitor.functions {
        match sol_fn_to_trait_method(func) {
            Ok(tokens) if tokens.is_empty() => {}
            Ok(tokens) => trait_fns.push(tokens),
            Err(err) => errors.push(err),
        }
    }

    if let Some(err) = combine_errors(errors) {
        return Err(err);
    }

    Ok((structs, trait_name, trait_fns))
}

/// Merges accumulated errors into a single one so a build reports every
/// unsupported item at once instead of only the first
///
/// # Arguments
///
/// * `errors` - The collected conversion errors
///
/// # Returns
///
/// The merged error, or `None` if there were no errors
fn combine_errors(errors: Vec<syn::Error>) -> Option<syn::Error> {
    errors.into_iter().reduce(|mut acc, err| {
        acc.combine(err);
        acc
    })
}

/// Derives a trait name from the Solidity file
///
/// # Arguments
///
/// * `file` - The Solidity file
///
/// # Returns
///
/// The derived trait name as an Ident or an error if no contract was found
fn derive_trait_name(file: &File) -> syn::Result<Ident> {
    for item in &file.items {
        if let Item::Contract(contract) = item {
            let name = contract.name.to_string().to_case(Case::Pascal);

            // If it's an interface, use the name directly
            if contract.kind.is_interface() {
                return Ok(format_ident!("{}", name));
            }

            // If it's a contract, prefix with 'I'
            if contract.kind.is_contract() {
                return Ok(format_ident!("I{}", name));
            }
        }
    }

    // Return error if no contract or interface is found
    Err(syn::Error::new(
        Span::call_site(),
        "No Solidity contract or interface found in input",
    ))
}

/// Determines the method receiver type based on mutability
///
/// # Arguments
///
/// * `func` - The Solidity function
///
/// # Returns
///
/// A TokenStream representing the receiver (&self or &mut self)
fn determine_method_receiver(func: &ItemFunction) -> TokenStream {
    match func.attributes.mutability() {
        Some(Mutability::View(_)) | Some(Mutability::Pure(_)) => quote! { &self },
        _ => quote! { &mut self },
    }
}

/// Converts a Solidity struct to Rust struct tokens
///
/// # Arguments
///
/// * `sol_struct` - The Solidity struct
///
/// # Returns
///
/// A TokenStream representing the generated Rust struct
fn sol_struct_to_rust_tokens(sol_struct: &ItemStruct) -> syn::Result<TokenStream> {
    let name = &sol_struct.name;

    // Convert all fields
    let fields = sol_struct
        .fields
        .iter()
        .map(|field| {
            // Get field name or use _ as default
            let field_name = field
                .name
                .as_ref()
                .map(|n| Ident::new(&n.to_string(), n.span()))
                .unwrap_or_else(|| Ident::new("_", field.ty.span()));

            // Convert Solidity type to Rust type
            let sol_ty = convert_solidity_type(&field.ty).map_err(|e| {
                syn::Error::new(field.ty.span(), format!("Struct field type error: {e}"))
            })?;
            let rust_ty: Type = sol_to_rust(&sol_ty).map_err(|e| {
                syn::Error::new(field.ty.span(), format!("Struct field type error: {e}"))
            })?;

            Ok(quote! { pub #field_name: #rust_ty })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    // Generate the struct definition
    Ok(quote! {
        #[derive(::fluentbase_sdk::codec::Codec, Debug, Clone, PartialEq, Eq)]
        pub struct #name {
            #(#fields),*
        }
    })
}

/// Converts a Solidity function to a Rust trait method
///
/// # Arguments
///
/// * `func` - The Solidity function
///
/// # Returns
///
/// A TokenStream representing the generated trait method
fn sol_fn_to_trait_method(func: &ItemFunction) -> syn::Result<TokenStream> {
    // Skip functions without a name or special functions
    let Some(name) = &func.name else {
        return Ok(quote! {});
    };

    if name == "fallback" || name == "receive" {
        return Ok(quote! {});
    }

    // Convert function name to snake_case
    let fn_name = format_ident!("{}", name.to_string().to_case(Case::Snake));
    let receiver = determine_method_receiver(func);

    // Generate function parameters. A dropped parameter would change the selector,
    // so any failure has to abort the whole function.
    let mut args = Vec::new();
    let mut errors = Vec::new();
    for (i, param) in func.parameters.iter().enumerate() {
        match sol_param_to_tokens(i, param) {
            Ok(tokens) => args.push(tokens),
            Err(err) => errors.push(err),
        }
    }

    // Generate function return type
    let ret = match sol_return_to_tokens(func) {
        Ok(tokens) => tokens,
        Err(err) => {
            errors.push(err);
            quote! {}
        }
    };

    if let Some(err) = combine_errors(errors) {
        return Err(err);
    }

    // Generate the function signature
    Ok(quote! {
        fn #fn_name(#receiver #(, #args)*) #ret;
    })
}

/// Converts a Solidity function parameter to Rust tokens
///
/// # Arguments
///
/// * `index` - The parameter index
/// * `param` - The parameter declaration
///
/// # Returns
///
/// A TokenStream representing the parameter
fn sol_param_to_tokens(index: usize, param: &VariableDeclaration) -> syn::Result<TokenStream> {
    // Get parameter name or generate one
    let name_str = param
        .name
        .as_ref()
        .map(|n| n.to_string().to_case(Case::Snake))
        .unwrap_or_else(|| format!("_param{index}"));

    let name_ident = format_ident!("{}", name_str);

    // Convert Solidity type to Rust type
    let sol_ty = convert_solidity_type(&param.ty)
        .map_err(|e| syn::Error::new(param.ty.span(), format!("Cannot convert param type: {e}")))?;
    let rust_ty = sol_to_rust(&sol_ty)
        .map_err(|e| syn::Error::new(param.ty.span(), format!("Cannot convert param type: {e}")))?;

    Ok(quote! { #name_ident: #rust_ty })
}

/// Converts a Solidity function return to Rust tokens
///
/// # Arguments
///
/// * `func` - The Solidity function
///
/// # Returns
///
/// A TokenStream representing the return type
fn sol_return_to_tokens(func: &ItemFunction) -> syn::Result<TokenStream> {
    // If there are no returns, return an empty token stream
    let Some(returns) = &func.returns else {
        return Ok(quote! {});
    };

    let return_params = &returns.returns;

    // If there's only one return parameter, use it directly
    if return_params.len() == 1 {
        let span = return_params[0].ty.span();
        let sol_ty = convert_solidity_type(&return_params[0].ty)
            .map_err(|e| syn::Error::new(span, format!("Return type error: {e}")))?;
        let rust_ty = sol_to_rust(&sol_ty)
            .map_err(|e| syn::Error::new(span, format!("Return type error: {e}")))?;

        return Ok(quote! { -> #rust_ty });
    }

    // For multiple return parameters, create a tuple
    let mut rust_types = Vec::new();
    let mut errors = Vec::new();
    for param in return_params.iter() {
        let converted = convert_solidity_type(&param.ty)
            .and_then(|sol_ty| sol_to_rust(&sol_ty))
            .map_err(|e| syn::Error::new(param.ty.span(), format!("Tuple return type error: {e}")));

        match converted {
            Ok(ty) => rust_types.push(quote! { #ty }),
            Err(err) => errors.push(err),
        }
    }

    if let Some(err) = combine_errors(errors) {
        return Err(err);
    }

    Ok(quote! { -> (#(#rust_types),*) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use prettyplease;
    use syn::parse_str;

    #[test]
    fn test_sol_struct_to_rust_tokens() {
        let sol = r#"
            struct User {
                string name;
                uint256 age;
            }
        "#;

        let item: syn_solidity::ItemStruct = parse_str(sol).unwrap();
        let tokens = sol_struct_to_rust_tokens(&item).unwrap();
        let file = syn::parse_file(&tokens.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("sol_struct_to_rust_tokens", formatted);
    }

    #[test]
    fn test_no_contract_error() {
        let solidity_code = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

library SomeLibrary {
    function doSomething() public pure returns (uint256) {
        return 42;
    }
}
"#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let result = to_rust_trait(input);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No Solidity contract or interface found"));
    }

    #[test]
    fn test_to_rust_trait_nested_struct() {
        let solidity_code = r#"
        // SPDX-License-Identifier: MIT
        pragma solidity ^0.8.0;

        interface IProgram {
            struct Inner {
                uint256 x;
            }

            struct Outer {
                Inner inner;
                string note;
            }

            function ping(Outer calldata input) external view returns (Outer calldata);
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let generated = to_rust_trait(input).unwrap();
        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("sol_to_rust_trait_nested_struct", formatted);
    }

    #[test]
    fn test_unsupported_param_type_fails() {
        let solidity_code = r#"
        interface IProgram {
            function transfer(address to, function() external cb) external;
            function ping() external view returns (uint256);
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let err = to_rust_trait(input).unwrap_err();
        assert!(
            err.to_string().contains("Cannot convert param type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_unsupported_return_type_fails() {
        let solidity_code = r#"
        interface IProgram {
            function lookup() external view returns (function() external);
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let err = to_rust_trait(input).unwrap_err();
        assert!(
            err.to_string().contains("Return type error"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_every_unsupported_item_is_reported() {
        let solidity_code = r#"
        interface IProgram {
            function first(function() external cb) external;
            function second(function() external cb) external;
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let err = to_rust_trait(input).unwrap_err();
        assert_eq!(err.into_iter().count(), 2);
    }

    #[test]
    fn test_supported_interface_keeps_all_functions_and_params() {
        let solidity_code = r#"
        interface IProgram {
            function transfer(address to, uint256 amount) external returns (bool);
            function balanceOf(address owner) external view returns (uint256);
            fallback() external;
            receive() external payable;
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let generated = to_rust_trait(input).unwrap();
        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("sol_to_rust_trait_full_surface", formatted);
    }

    #[test]
    fn test_inline_input_declares_no_file_dependency() {
        let solidity_code = r#"
        interface IProgram {
            function ping() external view returns (uint256);
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();
        assert!(input.path.is_none());

        let generated = to_rust_trait(input).unwrap().to_string();
        assert!(
            !generated.contains("include_str"),
            "inline input has no file to track: {generated}"
        );
    }

    #[test]
    fn test_to_sol_client_nested_struct() {
        let solidity_code = r#"
        // SPDX-License-Identifier: MIT
        pragma solidity ^0.8.0;

        interface IProgram {
            struct Inner {
                uint256 x;
            }

            struct Outer {
                Inner inner;
                string note;
            }

            function ping(Outer calldata input) external view returns (Outer calldata);
        }
    "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let generated = to_sol_client(input).unwrap();
        let parsed = syn::parse_file(&generated.to_string()).unwrap();
        let formatted = prettyplease::unparse(&parsed);

        assert_snapshot!("sol_to_sol_client_nested_struct", formatted);
    }
}

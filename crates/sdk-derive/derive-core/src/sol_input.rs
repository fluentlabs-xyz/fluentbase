use crate::{
    abi::{
        function::StateMutability,
        types::{convert_solidity_type, sol_to_rust},
    },
    attr::{StateMutabilityExt, STATE_MUTABILITY_ATTR},
    utils::naming::solidity_function_name,
};
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
    // A plain trait is compiled as-is, so it must not carry helper attributes.
    let (structs, trait_name, trait_fns) = convert_sol_to_rust(input, false)?;

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
    // The `client` macro consumes the trait, so mutability can be carried over
    // as an attribute and decide which host call each method issues.
    let (structs, trait_name, trait_fns) = convert_sol_to_rust(input, true)?;

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
/// `alloy_sol_macro_input` canonicalizes the path before reading it, so the
/// literal is absolute and does not resolve relative to the expansion site.
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
/// * `emit_mutability` - Whether to annotate methods with their Solidity state
///   mutability, which is only valid for traits consumed by a macro
///
/// # Returns
///
/// A tuple of (structs, trait_name, trait_methods) to be assembled
fn convert_sol_to_rust(
    input: SolInput,
    emit_mutability: bool,
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
        match sol_fn_to_trait_method(func, emit_mutability) {
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
    if sol_state_mutability(func).is_static() {
        quote! { &self }
    } else {
        quote! { &mut self }
    }
}

/// Reads the state mutability of a Solidity function.
///
/// It is carried over to the generated trait so client generation keeps issuing
/// the host call the Solidity declaration asks for, instead of defaulting every
/// method to a mutable `CALL` with a value.
///
/// # Arguments
///
/// * `func` - The Solidity function
///
/// # Returns
///
/// The state mutability of the function
fn sol_state_mutability(func: &ItemFunction) -> StateMutability {
    match func.attributes.mutability() {
        Some(Mutability::Pure(_)) => StateMutability::Pure,
        // `constant` is the legacy spelling of `view`
        Some(Mutability::View(_) | Mutability::Constant(_)) => StateMutability::View,
        Some(Mutability::Payable(_)) => StateMutability::Payable,
        None => StateMutability::NonPayable,
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
/// * `emit_mutability` - Whether to annotate the method with its Solidity state
///   mutability
///
/// # Returns
///
/// A TokenStream representing the generated trait method
fn sol_fn_to_trait_method(func: &ItemFunction, emit_mutability: bool) -> syn::Result<TokenStream> {
    // Skip functions without a name or special functions
    let Some(name) = &func.name else {
        return Ok(quote! {});
    };

    if name == "fallback" || name == "receive" {
        return Ok(quote! {});
    }

    // The Rust method is named in snake_case when that spelling converts back to the Solidity
    // one, which is what the router and client derive the selector from. A name it cannot
    // reproduce (`mintNFT`, `tokenURI`, `DOMAIN_SEPARATOR`) is kept verbatim: snake-casing it
    // would bake a selector no Solidity caller uses into the router and the generated client.
    let sol_name = name.to_string();
    let snake_name = sol_name.to_case(Case::Snake);
    let (fn_name, name_attr) = if solidity_function_name(&snake_name) == sol_name {
        (format_ident!("{}", snake_name), quote! {})
    } else if solidity_function_name(&sol_name) == sol_name {
        (
            format_ident!("{}", sol_name),
            quote! { #[allow(non_snake_case)] },
        )
    } else {
        // A name without uppercase letters that is not camelCase (`foo_bar`, `_foo`) is one no
        // Rust identifier derives: the router and client would camel-case it to `fooBar`.
        // Refusing it here beats compiling a trait whose selectors do not exist on chain.
        return Err(syn::Error::new(
            name.span(),
            format!(
                "Solidity function `{sol_name}` has no Rust spelling that derives its selector \
                 (the router and client would use `{}`); rename it in the interface, or write \
                 the trait by hand and pin `#[function_id(\"{sol_name}(...)\")]` on the \
                 implementation",
                solidity_function_name(&sol_name)
            ),
        ));
    };
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

    let mutability_attr = if emit_mutability {
        let mutability = sol_state_mutability(func).as_str();
        let attr = format_ident!("{}", STATE_MUTABILITY_ATTR);
        quote! { #[#attr(#mutability)] }
    } else {
        quote! {}
    };

    // Generate the function signature
    Ok(quote! {
        #mutability_attr
        #name_attr
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

    /// `mintNFT`, `tokenURI` and `DOMAIN_SEPARATOR` have no snake_case spelling that converts
    /// back to them, so the trait keeps the Solidity name; `balanceOf` round-trips and stays
    /// idiomatic. The generated client and a router over the trait derive the real selectors.
    #[test]
    fn test_mixed_case_names_keep_their_solidity_spelling() {
        let solidity_code = r#"
            interface INft {
                function mintNFT(address to) external;
                function tokenURI(uint256 id) external view returns (string memory);
                function DOMAIN_SEPARATOR() external view returns (bytes32);
                function balanceOf(address owner) external view returns (uint256);
            }
        "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let generated = to_rust_trait(input).unwrap().to_string();
        let file = syn::parse_file(&generated).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert!(
            formatted.contains("fn mintNFT(&mut self, to: Address);"),
            "{formatted}"
        );
        assert!(
            formatted.contains("fn tokenURI(&self, id: U256) -> String;"),
            "{formatted}"
        );
        assert!(
            formatted.contains("fn DOMAIN_SEPARATOR(&self) -> FixedBytes<32usize>;"),
            "{formatted}"
        );
        assert!(
            formatted.contains("fn balance_of(&self, owner: Address) -> U256;"),
            "{formatted}"
        );
        assert_eq!(
            formatted.matches("#[allow(non_snake_case)]").count(),
            3,
            "{formatted}"
        );

        // Each kept name derives the selector Solidity callers use.
        for (rust_name, selector) in [
            ("tokenURI(id: U256)", [0xc8, 0x7b, 0x56, 0xdd]),
            ("DOMAIN_SEPARATOR()", [0x36, 0x44, 0xe5, 0x15]),
            ("mintNFT(to: Address)", [0x54, 0xba, 0x0f, 0x27]),
        ] {
            let sig: syn::Signature = syn::parse_str(&format!("fn {rust_name}")).unwrap();
            let abi = crate::abi::function::FunctionABI::from_signature(&sig).unwrap();
            assert_eq!(abi.function_id().unwrap(), selector, "{rust_name}");
        }
    }

    /// `foo_bar` cannot be spelled by any Rust identifier the naming rule maps back to it, so
    /// the macro refuses it instead of deriving `fooBar`.
    #[test]
    fn test_lowercase_underscore_names_are_refused() {
        let solidity_code = r#"
            interface IOdd {
                function foo_bar(address to) external;
            }
        "#;
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        let err = to_rust_trait(input).unwrap_err().to_string();
        assert!(err.contains("`foo_bar`"), "{err}");
        assert!(err.contains("`fooBar`"), "{err}");
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

        for generated in [
            to_rust_trait(input.clone()).unwrap().to_string(),
            to_sol_client(input).unwrap().to_string(),
        ] {
            assert!(
                !generated.contains("include_str"),
                "inline input has no file to track: {generated}"
            );
        }
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

    /// Generates the client of a Solidity interface and strips whitespace, so
    /// assertions can pin the exact host call without depending on formatting
    fn sol_client_source(solidity_code: &str) -> String {
        let input: alloy_sol_macro_input::SolInput = parse_str(solidity_code).unwrap();

        // The trait produced here is what the `client` macro receives
        let trait_def: syn::ItemTrait = syn::parse2(to_sol_client(input).unwrap()).unwrap();
        let client = crate::client::Client::new(
            Default::default(),
            trait_def,
            &crate::abi::structs::StructResolver::default(),
        )
        .unwrap();

        client
            .generate()
            .unwrap()
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn test_solidity_mutability_decides_the_host_call() {
        let generated = sol_client_source(
            r#"
        interface IProgram {
            function answer() external pure returns (uint256);
            function balance() external view returns (uint256);
            function reset() external;
            function deposit() external payable;
        }
    "#,
        );

        // `pure` and `view` cannot mutate state, so they must be static calls
        // and must not be able to attach a value
        assert!(generated.contains(
            "pubfnanswer(&mutself,contract_address:fluentbase_sdk::Address,gas_limit:u64,)"
        ));
        assert!(generated.contains(
            "pubfnbalance(&mutself,contract_address:fluentbase_sdk::Address,gas_limit:u64,)"
        ));
        assert_eq!(
            generated
                .matches("self.sdk.static_call(contract_address,&input,Some(fuel_limit),)")
                .count(),
            2
        );

        // `nonpayable` mutates but rejects value, `payable` forwards it
        assert!(generated.contains(
            "pubfnreset(&mutself,contract_address:fluentbase_sdk::Address,gas_limit:u64,)"
        ));
        assert!(generated.contains(
            "self.sdk.call(contract_address,fluentbase_sdk::U256::ZERO,&input,Some(fuel_limit),)"
        ));
        assert!(generated.contains(
            "pubfndeposit(&mutself,contract_address:fluentbase_sdk::Address,value:fluentbase_sdk::U256,gas_limit:u64,)"
        ));
        assert!(
            generated.contains("self.sdk.call(contract_address,value,&input,Some(fuel_limit),)")
        );
    }

    #[test]
    fn test_legacy_constant_functions_are_read_only() {
        let generated = sol_client_source(
            r#"
        interface IProgram {
            function balance() external constant returns (uint256);
        }
    "#,
        );

        assert!(
            generated.contains("self.sdk.static_call(contract_address,&input,Some(fuel_limit),)")
        );
        assert!(!generated.contains("self.sdk.call("));
    }
}

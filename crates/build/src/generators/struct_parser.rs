//! Parser for extracting structs with Codec derive from Rust source files

use anyhow::{Context, Result};
use fluentbase_sdk_derive_core::abi::parameter::Parameter;
use serde_json::Value;
use std::{collections::HashMap, path::Path};
use syn::{
    parse::Parser, parse_file, punctuated::Punctuated, visit::Visit, Attribute, DeriveInput,
    ItemStruct, Meta, Path as SynPath, Token,
};

/// Parse structs from all .rs files in a directory
///
/// # Arguments
/// * `dir` - Path to the directory containing Rust source files
///
/// # Returns
/// * `HashMap<String, DeriveInput>` - Map of struct names to their parsed representations
pub fn parse_structs_from_dir(dir: &Path) -> Result<HashMap<String, DeriveInput>> {
    let mut all_structs = HashMap::new();

    // Walk through all .rs files in the directory
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            // Parse structs from this file
            match parse_structs(&path) {
                Ok(structs) => {
                    all_structs.extend(structs);
                }
                Err(e) => {
                    // Log warning but continue processing other files
                    eprintln!("Warning: Failed to parse structs from {path:?}: {e}");
                }
            }
        }
    }

    Ok(all_structs)
}

/// Parse structs with #[derive(Codec)] from a Rust source file
///
/// # Arguments
/// * `path` - Path to the Rust source file
///
/// # Returns
/// * `HashMap<String, DeriveInput>` - Map of struct names to their parsed representations
pub fn parse_structs(path: &Path) -> Result<HashMap<String, DeriveInput>> {
    // Read file content
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    // Parse Rust syntax
    let ast =
        parse_file(&content).map_err(|e| anyhow::anyhow!("Failed to parse Rust file: {}", e))?;

    // Find structs with Codec derive
    let mut collector = StructCollector::new();
    collector.visit_file(&ast);

    Ok(collector.structs)
}

/// Enrich a complete ABI entry (function) with struct components
///
/// # Arguments
/// * `entry` - Mutable reference to the ABI entry (JSON value)
/// * `structs` - Registry of parsed struct definitions
///
/// # Returns
/// * `Result<()>` - Ok if enrichment succeeded
pub fn enrich_abi_entry(entry: &mut Value, structs: &HashMap<String, DeriveInput>) -> Result<()> {
    // Enrich inputs
    if let Some(inputs) = entry.get_mut("inputs") {
        enrich_parameters(inputs, structs)?;
    }

    // Enrich outputs
    if let Some(outputs) = entry.get_mut("outputs") {
        enrich_parameters(outputs, structs)?;
    }

    Ok(())
}

/// Enrich parameters in ABI with struct component information
///
/// # Arguments
/// * `params` - Mutable reference to parameters array (JSON value)
/// * `structs` - Registry of parsed struct definitions
///
/// # Returns
/// * `Result<()>` - Ok if enrichment succeeded
pub fn enrich_parameters(params: &mut Value, structs: &HashMap<String, DeriveInput>) -> Result<()> {
    // params should be an array of parameters
    if let Some(params_array) = params.as_array_mut() {
        for param in params_array.iter_mut() {
            enrich_single_parameter(param, structs)?;
        }
    }
    Ok(())
}

/// Visitor for collecting structs with #[derive(Codec)]
struct StructCollector {
    structs: HashMap<String, DeriveInput>,
}

impl StructCollector {
    fn new() -> Self {
        Self {
            structs: HashMap::new(),
        }
    }

    /// Check if attributes contain #[derive(...)] with Codec
    /// Returns true if attributes contain `#[derive(Codec)]`
    fn has_codec_derive(attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| match &attr.meta {
            Meta::List(list) if list.path.is_ident("derive") => {
                let derives = Punctuated::<SynPath, Token![,]>::parse_terminated
                    .parse2(list.tokens.clone())
                    .ok();

                derives
                    .map(|d| d.iter().any(|p| p.is_ident("Codec")))
                    .unwrap_or(false)
            }
            _ => false,
        })
    }

    /// Convert ItemStruct to DeriveInput
    fn item_struct_to_derive_input(item: &ItemStruct) -> DeriveInput {
        DeriveInput {
            attrs: item.attrs.clone(),
            vis: item.vis.clone(),
            ident: item.ident.clone(),
            generics: item.generics.clone(),
            data: syn::Data::Struct(syn::DataStruct {
                struct_token: item.struct_token,
                fields: item.fields.clone(),
                semi_token: item.semi_token,
            }),
        }
    }
}

impl<'ast> Visit<'ast> for StructCollector {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        // Check if this struct has #[derive(Codec)]
        if Self::has_codec_derive(&node.attrs) {
            let struct_name = node.ident.to_string();
            let derive_input = Self::item_struct_to_derive_input(node);

            // Store the struct for later use
            self.structs.insert(struct_name, derive_input);
        }

        // Continue visiting nested items
        syn::visit::visit_item_struct(self, node);
    }
}

/// Enrich a single parameter with struct components if applicable
fn enrich_single_parameter(
    param: &mut Value,
    structs: &HashMap<String, DeriveInput>,
) -> Result<()> {
    // Check if this parameter is a struct (tuple with struct internal type)
    if param["type"] == "tuple" {
        if let Some(internal_type) = param.get("internalType").and_then(Value::as_str) {
            if let Some(struct_name) = internal_type.strip_prefix("struct ") {
                // Found a struct parameter, look it up in our registry
                if let Some(derive_input) = structs.get(struct_name) {
                    // Use Parameter::from_derive_input to get proper components
                    match Parameter::from_derive_input(derive_input) {
                        Ok(param_with_components) => {
                            // Serialize the Parameter to JSON to extract components
                            if let Ok(param_json) = serde_json::to_value(&param_with_components) {
                                // Replace empty components with the correct ones
                                if let Some(components) = param_json.get("components") {
                                    param["components"] = components.clone();
                                }
                            }
                        }
                        Err(e) => {
                            // Log warning but continue processing
                            eprintln!("Warning: Failed to enrich struct {struct_name}: {e:?}");
                        }
                    }
                }
            }
        }

        // Recursively process nested components (for nested structs)
        if let Some(components) = param.get_mut("components").and_then(Value::as_array_mut) {
            for component in components.iter_mut() {
                enrich_single_parameter(component, structs)?;
            }
        }
    }
    // FIX: Handle tuple[] (arrays of structs)
    else if param["type"] == "tuple[]" {
        // For tuple arrays, check if it's an array of structs
        if let Some(internal_type) = param.get("internalType").and_then(Value::as_str) {
            if let Some(struct_name) = internal_type
                .strip_prefix("struct ")
                .and_then(|s| s.strip_suffix("[]"))
            {
                // This is an array of structs
                if let Some(derive_input) = structs.get(struct_name) {
                    match Parameter::from_derive_input(derive_input) {
                        Ok(param_with_components) => {
                            if let Ok(param_json) = serde_json::to_value(&param_with_components) {
                                if let Some(components) = param_json.get("components") {
                                    param["components"] = components.clone();
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to enrich struct array {struct_name}: {e:?}"
                            );
                        }
                    }
                }
            }
        }

        // Recursively process components if they exist
        if let Some(components) = param.get_mut("components").and_then(Value::as_array_mut) {
            for component in components.iter_mut() {
                enrich_single_parameter(component, structs)?;
            }
        }
    }
    // Original handling for old-style arrays (kept for compatibility)
    else if param["type"]
        .as_str()
        .map(|s| s.ends_with("[]"))
        .unwrap_or(false)
    {
        // Handle arrays of structs (backward compatibility)
        if let Some(internal_type) = param.get("internalType").and_then(Value::as_str) {
            if let Some(struct_name) = internal_type
                .strip_prefix("struct ")
                .and_then(|s| s.strip_suffix("[]"))
            {
                // This is an array of structs
                if let Some(derive_input) = structs.get(struct_name) {
                    match Parameter::from_derive_input(derive_input) {
                        Ok(param_with_components) => {
                            if let Ok(param_json) = serde_json::to_value(&param_with_components) {
                                if let Some(components) = param_json.get("components") {
                                    param["components"] = components.clone();
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to enrich struct array {struct_name}: {e:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    mod parse {
        use crate::generators::struct_parser::parse_structs;
        use std::fs;
        use tempfile::TempDir;

        /// Helper to create a temporary Rust file
        fn create_temp_rust_file(content: &str) -> (TempDir, std::path::PathBuf) {
            let temp_dir = TempDir::new().unwrap();
            let file_path = temp_dir.path().join("test.rs");
            fs::write(&file_path, content).unwrap();
            (temp_dir, file_path)
        }

        #[test]
        fn test_parse_simple_struct_with_codec() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

#[derive(Codec, Debug, Clone)]
pub struct TestStruct {
    pub field1: U256,
    pub field2: bool,
    pub field3: Address,
}

#[derive(Debug, Clone)]
pub struct StructWithoutCodec {
    pub field1: u32,
}

#[derive(Codec)]
pub struct AnotherCodecStruct {
    pub value: U256,
}
"#;

            let (_temp_dir, file_path) = create_temp_rust_file(content);
            let structs = parse_structs(&file_path).unwrap();

            // Should find exactly 2 structs with Codec
            assert_eq!(structs.len(), 2);

            // Check that we found the right structs
            assert!(structs.contains_key("TestStruct"));
            assert!(structs.contains_key("AnotherCodecStruct"));
            assert!(!structs.contains_key("StructWithoutCodec"));

            // Verify the TestStruct has correct fields
            let test_struct = &structs["TestStruct"];
            if let syn::Data::Struct(data) = &test_struct.data {
                let field_names: Vec<String> = data
                    .fields
                    .iter()
                    .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                    .collect();

                assert_eq!(field_names, vec!["field1", "field2", "field3"]);
            } else {
                panic!("Expected struct data");
            }
        }

        #[test]
        fn test_parse_nested_structs() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

mod inner {
    use super::*;

    #[derive(Codec)]
    pub struct InnerStruct {
        pub value: U256,
    }
}

#[derive(Codec, Debug)]
pub struct OuterStruct {
    pub inner: inner::InnerStruct,
    pub data: U256,
}
"#;

            let (_temp_dir, file_path) = create_temp_rust_file(content);
            let structs = parse_structs(&file_path).unwrap();

            // Should find both inner and outer structs
            assert_eq!(structs.len(), 2);
            assert!(structs.contains_key("InnerStruct"));
            assert!(structs.contains_key("OuterStruct"));
        }

        #[test]
        fn test_empty_file() {
            let content = r#"
// Empty file with no structs
use fluentbase_sdk::codec::Codec;
"#;

            let (_temp_dir, file_path) = create_temp_rust_file(content);
            let structs = parse_structs(&file_path).unwrap();

            assert_eq!(structs.len(), 0);
        }

        #[test]
        fn test_struct_with_unnamed_fields() {
            let content = r#"
use fluentbase_sdk::codec::Codec;
use fluentbase_sdk::U256;

#[derive(Codec)]
pub struct TupleStruct(pub U256, pub bool);

#[derive(Codec)]
pub struct UnitStruct;
"#;

            let (_temp_dir, file_path) = create_temp_rust_file(content);
            let structs = parse_structs(&file_path).unwrap();

            assert_eq!(structs.len(), 2);
            assert!(structs.contains_key("TupleStruct"));
            assert!(structs.contains_key("UnitStruct"));
        }
    }
    mod enrich {
        use crate::generators::struct_parser::enrich_parameters;
        use serde_json::json;
        use std::collections::HashMap;
        use syn::{parse_quote, DeriveInput};

        /// Helper to create a test DeriveInput for a struct
        fn create_test_struct(name: &str, fields: Vec<(&str, &str)>) -> DeriveInput {
            use quote::format_ident;

            let ident = format_ident!("{}", name);

            // Build the field list directly in the parse_quote macro
            let field_tokens = fields
                .into_iter()
                .map(|(field_name, field_type)| {
                    let field_ident = format_ident!("{}", field_name);
                    let type_ident = format_ident!("{}", field_type);
                    quote::quote! {
                        pub #field_ident: #type_ident
                    }
                })
                .collect::<Vec<_>>();

            parse_quote! {
                #[derive(Codec)]
                pub struct #ident {
                    #(#field_tokens),*
                }
            }
        }

        #[test]
        fn test_enrich_simple_struct_parameter() {
            // Create a struct registry with SlippageParams
            let mut structs = HashMap::new();
            structs.insert(
                "SlippageParams".to_string(),
                create_test_struct(
                    "SlippageParams",
                    vec![
                        ("amount_in", "U256"),
                        ("reserve_in", "U256"),
                        ("reserve_out", "U256"),
                        ("fee_rate", "U256"),
                    ],
                ),
            );

            // Create a parameter with empty components
            let mut params = json!([{
                "name": "params",
                "type": "tuple",
                "internalType": "struct SlippageParams",
                "components": []  // Empty components to be filled
            }]);

            // Enrich the parameters
            enrich_parameters(&mut params, &structs).unwrap();

            // Check that components were added
            let components = params[0]["components"].as_array().unwrap();
            assert_eq!(components.len(), 4, "Should have 4 components");

            // Verify field names
            assert_eq!(components[0]["name"], "amount_in");
            assert_eq!(components[1]["name"], "reserve_in");
            assert_eq!(components[2]["name"], "reserve_out");
            assert_eq!(components[3]["name"], "fee_rate");

            // Verify field types (U256 should map to uint256)
            assert_eq!(components[0]["type"], "uint256");
            assert_eq!(components[1]["type"], "uint256");
            assert_eq!(components[2]["type"], "uint256");
            assert_eq!(components[3]["type"], "uint256");
        }
    }
}

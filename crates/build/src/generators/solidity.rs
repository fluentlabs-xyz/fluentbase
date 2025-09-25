//! Solidity ABI and interface generation from Rust smart contracts

use crate::generators::struct_parser::{enrich_abi_entry, parse_structs_from_dir};
use anyhow::{Context, Result};
use convert_case::{Case, Casing};
use fluentbase_sdk_derive_core::{
    constructor::{process_constructor, Constructor},
    router::{process_router, Router},
};
use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use syn::{parse_file, visit::Visit, Attribute, DeriveInput, ItemImpl};

/// Solidity ABI represented as JSON values
pub type Abi = Vec<Value>;

/// Parse contract directory and generate ABI
///
/// # Arguments
/// * `contract_dir` - Path to the contract directory (contains src/ with lib.rs or main.rs)
///
/// # Returns
/// * `Result<Abi>` - JSON array with function definitions or empty array if no functions found
pub fn generate_abi(contract_dir: &Path) -> Result<Abi> {
    // Find the main source file
    let src_dir = contract_dir.join("src");
    let main_file = if src_dir.join("lib.rs").exists() {
        src_dir.join("lib.rs")
    } else if src_dir.join("main.rs").exists() {
        src_dir.join("main.rs")
    } else {
        return Err(anyhow::anyhow!(
            "No lib.rs or main.rs found in {}",
            src_dir.display()
        ));
    };

    // Parse all structs from the src directory
    let structs = parse_structs_from_dir(&src_dir)?;

    // Parse contract methods (routers and constructors) from the main file
    let methods = parse_contract_methods(&main_file)?;

    // Generate ABI from contract methods with struct enrichment
    generate_abi_from_methods(&methods, &structs)
}

/// Generate Solidity interface from ABI
///
/// # Arguments
/// * `contract_name` - Name of the contract (used for interface name)
/// * `abi` - Previously generated ABI
///
/// # Returns
/// * `Result<String>` - Solidity interface code
pub fn generate_interface(contract_name: &str, abi: &Abi) -> Result<String> {
    let mut interface = String::new();

    // Header
    interface.push_str("// SPDX-License-Identifier: MIT\n");
    interface.push_str("// Auto-generated from Rust source\n");
    interface.push_str("pragma solidity ^0.8.0;\n\n");
    interface.push_str(&format!(
        "interface I{} {{\n",
        contract_name.to_case(Case::Pascal)
    ));

    // Extract and add struct definitions
    let mut seen_structs = HashSet::new();
    let mut struct_definitions = Vec::new();

    // Collect structs from both constructor and functions
    for entry in abi {
        if let Some(inputs) = entry.get("inputs").and_then(Value::as_array) {
            collect_structs(inputs, &mut seen_structs, &mut struct_definitions);
        }
        if let Some(outputs) = entry.get("outputs").and_then(Value::as_array) {
            collect_structs(outputs, &mut seen_structs, &mut struct_definitions);
        }
    }

    // Add structs to interface
    if !struct_definitions.is_empty() {
        for struct_def in &struct_definitions {
            interface.push_str(struct_def);
            interface.push_str("\n\n");
        }
    }

    // Note: Constructors are not included in interfaces
    // Add only functions to the interface
    for func in abi.iter().filter(|e| e["type"] == "function") {
        interface.push_str("    ");
        interface.push_str(&format_function(func)?);
        interface.push('\n');
    }

    interface.push_str("}\n");
    Ok(interface)
}

// Internal types and functions

/// Container for all contract elements found during parsing
struct ContractMethods {
    constructor: Option<Constructor>,
    routers: Vec<Router>,
}

/// Parses a Rust file and extracts all contract elements (routers and constructors)
fn parse_contract_methods(path: &Path) -> Result<ContractMethods> {
    // Read file content
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    // Parse Rust syntax
    let ast =
        parse_file(&content).map_err(|e| anyhow::anyhow!("Failed to parse Rust file: {}", e))?;

    // Find contract methods
    let mut finder = ContractMethodFinder::new();
    finder.visit_file(&ast);

    // Return first error if any occurred during processing
    if let Some(error) = finder.errors.into_iter().next() {
        return Err(anyhow::anyhow!("Contract parsing error: {}", error));
    }

    Ok(ContractMethods {
        constructor: finder.constructor,
        routers: finder.routers,
    })
}

/// Generates ABI from parsed contract methods with struct enrichment
fn generate_abi_from_methods(
    methods: &ContractMethods,
    structs: &HashMap<String, DeriveInput>,
) -> Result<Abi> {
    let mut entries = Vec::new();

    // Process constructor first (they appear first in standard ABIs)
    if let Some(constructor) = &methods.constructor {
        let constructor_method = constructor.constructor_method();
        if let Ok(constructor_abi) = constructor_method.parsed_signature().constructor_abi() {
            if let Ok(mut json) = constructor_abi.to_json_value() {
                // Enrich the ABI entry with struct components
                enrich_abi_entry(&mut json, structs)?;
                entries.push(json);
            }
        }
    }

    // Process routers - take first router if multiple exist
    if let Some(router) = methods.routers.first() {
        // Check if router has a constructor (for backward compatibility)
        // Skip it if we already processed standalone constructors
        if methods.constructor.is_none() {
            if let Some(constructor) = router.constructor() {
                if let Ok(constructor_abi) = constructor.parsed_signature().constructor_abi() {
                    if let Ok(mut json) = constructor_abi.to_json_value() {
                        enrich_abi_entry(&mut json, structs)?;
                        entries.push(json);
                    }
                }
            }
        }

        // Add all functions from the router
        for method in router.available_methods() {
            if let Ok(func_abi) = method.parsed_signature().function_abi() {
                if let Ok(mut json) = func_abi.to_json_value() {
                    enrich_abi_entry(&mut json, structs)?;
                    entries.push(json);
                }
            }
        }
    }

    Ok(entries)
}

/// Internal visitor for finding contract elements (routers and constructors)
struct ContractMethodFinder {
    routers: Vec<Router>,
    constructor: Option<Constructor>,
    errors: Vec<syn::Error>,
}

impl ContractMethodFinder {
    fn new() -> Self {
        Self {
            routers: Vec::new(),
            constructor: None,
            errors: Vec::new(),
        }
    }

    fn process_router_impl(&mut self, attr: &Attribute, impl_block: &ItemImpl) {
        match extract_attribute_tokens(attr) {
            Ok(attr_tokens) => match process_router(attr_tokens, impl_block.to_token_stream()) {
                Ok(router) => self.routers.push(router),
                Err(error) => self.errors.push(error),
            },
            Err(error) => self.errors.push(error),
        }
    }

    fn process_constructor_impl(&mut self, attr: &Attribute, impl_block: &ItemImpl) {
        match extract_attribute_tokens(attr) {
            Ok(attr_tokens) => {
                match process_constructor(attr_tokens, impl_block.to_token_stream()) {
                    Ok(constructor) => self.constructor = Some(constructor),
                    Err(error) => self.errors.push(error),
                }
            }
            Err(error) => self.errors.push(error),
        }
    }
}

impl<'ast> Visit<'ast> for ContractMethodFinder {
    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        // Look for router or constructor attributes
        for attr in &node.attrs {
            if is_router_attribute(attr) {
                self.process_router_impl(attr, node);
                break; // Found router attribute - no need to check other attributes
            } else if is_constructor_attribute(attr) {
                self.process_constructor_impl(attr, node);
                break; // Found constructor attribute - no need to check other attributes
            }
        }

        // Continue visiting nested items
        syn::visit::visit_item_impl(self, node);
    }
}

/// Checks if an attribute is a router attribute
fn is_router_attribute(attr: &Attribute) -> bool {
    attr.path().is_ident("router")
}

/// Checks if an attribute is a constructor attribute
fn is_constructor_attribute(attr: &Attribute) -> bool {
    attr.path().is_ident("constructor")
}

/// Extracts tokens from an attribute (works for both router and constructor)
fn extract_attribute_tokens(attr: &Attribute) -> syn::Result<TokenStream2> {
    match &attr.meta {
        syn::Meta::List(meta_list) => Ok(meta_list.tokens.clone()),
        syn::Meta::Path(_) => Ok(TokenStream2::new()), // Attribute without parameters
        _ => Err(syn::Error::new_spanned(
            attr,
            "Invalid attribute format. Expected #[router] or #[constructor] with optional parameters",
        )),
    }
}

#[derive(Debug, Clone, Copy)]
enum ParameterKind {
    Input,
    Output,
}

fn format_function(func: &Value) -> Result<String> {
    let name = func["name"].as_str().unwrap_or_default();
    let empty_vec = Vec::new();
    let inputs = func["inputs"].as_array().unwrap_or(&empty_vec);
    let outputs = func["outputs"].as_array().unwrap_or(&empty_vec);
    let mutability = func["stateMutability"].as_str().unwrap_or("nonpayable");

    // Format input parameters with calldata location for external functions
    let params = inputs
        .iter()
        .map(|p| format_parameter(p, ParameterKind::Input))
        .collect::<Vec<_>>()
        .join(", ");

    // Format output parameters with memory location
    let returns = if outputs.is_empty() {
        String::new()
    } else {
        let ret_params = outputs
            .iter()
            .map(|p| format_parameter(p, ParameterKind::Output))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" returns ({ret_params})")
    };

    let mut_str = match mutability {
        "pure" => " pure",
        "view" => " view",
        "payable" => " payable",
        _ => "",
    };

    Ok(format!(
        "function {name}({params}) external{mut_str}{returns};"
    ))
}

fn format_parameter(param: &Value, param_kind: ParameterKind) -> String {
    let name = param["name"].as_str().unwrap_or("");
    let internal_type = param.get("internalType").and_then(Value::as_str);

    // Use internal type for structs, otherwise use regular type
    let ty = if let Some(internal) = internal_type {
        if let Some(struct_name) = internal.strip_prefix("struct ") {
            struct_name.to_string()
        } else {
            format_sol_type(param)
        }
    } else {
        format_sol_type(param)
    };

    // Add data location for complex types
    let location = get_data_location(&ty, internal_type, param_kind);
    let location_str = match location {
        Some(DataLocation::Memory) => " memory",
        Some(DataLocation::Calldata) => " calldata",
        None => "",
    };

    if name.is_empty() {
        format!("{ty}{location_str}")
    } else {
        format!("{ty}{location_str} {name}")
    }
}

fn format_sol_type(param: &Value) -> String {
    let param_type = param["type"].as_str().unwrap_or("unknown");

    // Handle all tuple types (including multidimensional arrays)
    if let Some(array_suffix) = param_type.strip_prefix("tuple") {
        // Check if it's a named struct
        if let Some(internal_type) = param.get("internalType").and_then(Value::as_str) {
            if let Some(stripped) = internal_type.strip_prefix("struct ") {
                // Return the struct name with all array suffixes preserved
                return stripped.to_string();
            }
        }

        // Handle anonymous tuples
        if param_type == "tuple" {
            // Simple tuple
            if let Some(components) = param.get("components").and_then(Value::as_array) {
                let component_types = components
                    .iter()
                    .map(format_sol_type)
                    .collect::<Vec<_>>()
                    .join(",");
                return format!("({component_types})");
            }
        } else if param_type.starts_with("tuple[") && param_type.ends_with("]") {
            // Tuple array (any dimensionality)
            if let Some(components) = param.get("components").and_then(Value::as_array) {
                let component_types = components
                    .iter()
                    .map(format_sol_type)
                    .collect::<Vec<_>>()
                    .join(",");
                return format!("({component_types}){array_suffix}");
            }
        }

        // Fallback
        param_type.to_string()
    } else if let Some(base_type) = param_type.strip_suffix("[]") {
        // Handle other array types (not tuple arrays)
        let formatted_base = format_sol_type(&serde_json::json!({ "type": base_type }));
        format!("{formatted_base}[]")
    } else {
        // Return primitive types as-is
        param_type.to_string()
    }
}

#[derive(Debug, Clone, Copy)]
enum DataLocation {
    Memory,
    Calldata,
}

fn get_data_location(
    ty: &str,
    internal_type: Option<&str>,
    param_kind: ParameterKind,
) -> Option<DataLocation> {
    // For external functions in interfaces:
    // - Input parameters: use calldata (more gas efficient, read-only)
    // - Output parameters: use memory

    // Simple types (uint256, address, bool, etc.) don't need data location
    if !needs_data_location(ty, internal_type) {
        return None;
    }

    match param_kind {
        ParameterKind::Input => Some(DataLocation::Calldata),
        ParameterKind::Output => Some(DataLocation::Memory),
    }
}

fn needs_data_location(ty: &str, internal_type: Option<&str>) -> bool {
    // Data location is needed for:
    // - Structs
    // - Arrays (dynamic and fixed, any dimensionality)
    // - Strings
    // - Bytes (dynamic)
    // - Tuples

    match (ty, internal_type) {
        (_, Some(t)) if t.starts_with("struct ") => true,
        ("string", _) | ("bytes", _) => true,
        (t, _) if t.contains("[") && t.contains("]") => true, // Any array (including multidimensional)
        (t, _) if t.starts_with("(") && t.ends_with(")") => true, // Tuples
        _ => false,
    }
}

/// Helper function to strip all array suffixes from a string
/// Example: "Cell[][]" -> "Cell", "Item[3][2]" -> "Item"
fn strip_array_suffixes(s: &str) -> &str {
    let mut result = s;
    // Handle both dynamic arrays [] and fixed arrays [n]
    while let Some(bracket_pos) = result.rfind('[') {
        result = &result[..bracket_pos];
    }
    result
}

fn collect_structs(params: &[Value], seen: &mut HashSet<String>, structs: &mut Vec<String>) {
    for param in params {
        let param_type = param["type"].as_str().unwrap_or("");

        // Check if this parameter represents a struct (tuple or any tuple array)
        let is_tuple_type = param_type == "tuple"
            || (param_type.starts_with("tuple[") && param_type.ends_with("]"));

        if is_tuple_type {
            if let Some(internal_type) = param.get("internalType").and_then(Value::as_str) {
                // Extract struct name from "struct Name" or "struct Name[]" or "struct Name[][]" etc.
                if let Some(struct_name_with_arrays) = internal_type.strip_prefix("struct ") {
                    // Remove all array suffixes to get the pure struct name
                    let struct_name = strip_array_suffixes(struct_name_with_arrays);

                    if seen.insert(struct_name.to_string()) {
                        if let Some(components) = param.get("components").and_then(Value::as_array)
                        {
                            let fields = components
                                .iter()
                                .map(|field| {
                                    let field_name = field["name"].as_str().unwrap_or("_");
                                    let field_type = format_sol_type(field);
                                    format!("        {field_type} {field_name};")
                                })
                                .collect::<Vec<_>>()
                                .join("\n");

                            structs.push(format!("    struct {struct_name} {{\n{fields}\n    }}"));

                            // Recursively collect nested structs
                            collect_structs(components, seen, structs);
                        }
                    }
                }
            }
        } else if param_type.ends_with("]") {
            // For other array types, just check components recursively
            if let Some(components) = param.get("components").and_then(Value::as_array) {
                collect_structs(components, seen, structs);
            }
        } else if param_type == "tuple" {
            // For anonymous tuples, check components
            if let Some(components) = param.get("components").and_then(Value::as_array) {
                collect_structs(components, seen, structs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_interface_simple() {
        let abi = vec![
            serde_json::json!({
                "name": "transfer",
                "type": "function",
                "inputs": [
                    {"name": "to", "type": "address", "internalType": "address"},
                    {"name": "amount", "type": "uint256", "internalType": "uint256"}
                ],
                "outputs": [{"name": "", "type": "bool", "internalType": "bool"}],
                "stateMutability": "nonpayable"
            }),
            serde_json::json!({
                "name": "balanceOf",
                "type": "function",
                "inputs": [
                    {"name": "account", "type": "address", "internalType": "address"}
                ],
                "outputs": [{"name": "", "type": "uint256", "internalType": "uint256"}],
                "stateMutability": "view"
            }),
        ];

        let interface = generate_interface("ERC20Token", &abi).unwrap();

        assert!(interface.contains("interface IErc20Token"));
        assert!(interface
            .contains("function transfer(address to, uint256 amount) external returns (bool);"));
        assert!(interface
            .contains("function balanceOf(address account) external view returns (uint256);"));
    }

    #[test]
    fn test_generate_interface_with_structs() {
        let abi = vec![serde_json::json!({
            "name": "submitOrder",
            "type": "function",
            "inputs": [{
                "name": "order",
                "type": "tuple",
                "internalType": "struct Order",
                "components": [
                    {"name": "id", "type": "uint256", "internalType": "uint256"},
                    {"name": "user", "type": "address", "internalType": "address"},
                    {"name": "metadata", "type": "bytes", "internalType": "bytes"}
                ]
            }],
            "outputs": [{"name": "success", "type": "bool", "internalType": "bool"}],
            "stateMutability": "payable"
        })];

        let interface = generate_interface("OrderManager", &abi).unwrap();
        assert!(interface.contains("struct Order"));
        assert!(interface.contains("uint256 id;"));
        assert!(interface.contains("address user;"));
        assert!(interface.contains("bytes metadata;"));
        assert!(interface.contains(
            "function submitOrder(Order calldata order) external payable returns (bool success);"
        ));
    }

    #[test]
    fn test_generate_interface_with_constructor() {
        let abi = vec![
            serde_json::json!({
                "type": "constructor",
                "inputs": [
                    {"name": "owner", "type": "address", "internalType": "address"},
                    {"name": "initialSupply", "type": "uint256", "internalType": "uint256"}
                ],
                "stateMutability": "nonpayable"
            }),
            serde_json::json!({
                "name": "transfer",
                "type": "function",
                "inputs": [
                    {"name": "to", "type": "address", "internalType": "address"},
                    {"name": "amount", "type": "uint256", "internalType": "uint256"}
                ],
                "outputs": [{"name": "", "type": "bool", "internalType": "bool"}],
                "stateMutability": "nonpayable"
            }),
        ];

        let interface = generate_interface("Token", &abi).unwrap();

        // Constructor should not appear in interface
        assert!(!interface.contains("constructor"));

        // But function should appear
        assert!(interface
            .contains("function transfer(address to, uint256 amount) external returns (bool);"));
    }

    #[test]
    fn test_generate_interface_empty_abi() {
        let abi = vec![];
        let interface = generate_interface("EmptyContract", &abi).unwrap();
        assert!(interface.contains("interface IEmptyContract {"));
        assert!(interface.contains("}\n"));
    }

    #[test]
    fn test_generate_interface_all_mutabilities() {
        let abi = vec![
            serde_json::json!({
                "name": "pureFunction",
                "type": "function",
                "inputs": [{"name": "x", "type": "uint256", "internalType": "uint256"}],
                "outputs": [{"name": "", "type": "uint256", "internalType": "uint256"}],
                "stateMutability": "pure"
            }),
            serde_json::json!({
                "name": "viewFunction",
                "type": "function",
                "inputs": [],
                "outputs": [{"name": "", "type": "string", "internalType": "string"}],
                "stateMutability": "view"
            }),
            serde_json::json!({
                "name": "payableFunction",
                "type": "function",
                "inputs": [{"name": "data", "type": "bytes", "internalType": "bytes"}],
                "outputs": [],
                "stateMutability": "payable"
            }),
            serde_json::json!({
                "name": "nonpayableFunction",
                "type": "function",
                "inputs": [],
                "outputs": [],
                "stateMutability": "nonpayable"
            }),
        ];

        let interface = generate_interface("MixedContract", &abi).unwrap();
        assert!(
            interface.contains("function pureFunction(uint256 x) external pure returns (uint256);")
        );
        assert!(
            interface.contains("function viewFunction() external view returns (string memory);")
        );
        assert!(
            interface.contains("function payableFunction(bytes calldata data) external payable;")
        );
        assert!(interface.contains("function nonpayableFunction() external;"));
    }
}

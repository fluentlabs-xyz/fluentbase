use crate::abi::{constructor::ConstructorABI, function::FunctionABI};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a stored ABI definition with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractABI {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constructor: Option<ConstructorABI>,
    pub functions: Vec<FunctionABI>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

#[allow(dead_code)]
impl ContractABI {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            constructor: None,
            functions: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_constructor(mut self, constructor: ConstructorABI) -> Self {
        self.constructor = Some(constructor);
        self
    }

    #[must_use]
    pub fn with_function(mut self, function: FunctionABI) -> Self {
        self.functions.push(function);
        self
    }

    #[must_use]
    pub fn with_functions(mut self, functions: Vec<FunctionABI>) -> Self {
        self.functions.extend(functions);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn path(&self) -> String {
        self.id.replace("::", "/")
    }

    /// Converts to standard Solidity ABI format (array of all entries)
    pub fn to_solidity_abi(&self) -> Vec<serde_json::Value> {
        let mut abi = Vec::new();

        // Add constructor first if present
        if let Some(ref constructor) = self.constructor {
            if let Ok(value) = constructor.to_json_value() {
                abi.push(value);
            }
        }

        // Add all functions
        for function in &self.functions {
            if let Ok(value) = function.to_json_value() {
                abi.push(value);
            }
        }

        abi
    }

    /// Exports to standard JSON ABI format
    pub fn to_json_abi(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.to_solidity_abi())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_contract_with_constructor() {
        let constructor_sig: syn::Signature = parse_quote! {
            fn constructor(admin: Address, supply: U256)
        };

        let function_sig: syn::Signature = parse_quote! {
            fn transfer(to: Address, amount: U256) -> bool
        };

        let constructor_abi = ConstructorABI::from_signature(&constructor_sig).unwrap();
        let function_abi = FunctionABI::from_signature(&function_sig).unwrap();

        let contract = ContractABI::new("MyToken")
            .with_constructor(constructor_abi)
            .with_function(function_abi);

        assert!(contract.constructor.is_some());
        assert_eq!(contract.functions.len(), 1);

        let abi_array = contract.to_solidity_abi();
        assert_eq!(abi_array.len(), 2);

        // Constructor should be first
        assert_eq!(abi_array[0]["type"], "constructor");
        assert_eq!(abi_array[1]["type"], "function");
    }
}

use crate::abi::function::FunctionABI;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a stored ABI definition with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractABI {
    pub id: String,
    pub functions: Vec<FunctionABI>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

#[allow(dead_code)]
impl ContractABI {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            functions: Vec::new(),
            metadata: HashMap::new(),
        }
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
}

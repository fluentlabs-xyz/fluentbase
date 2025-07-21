//! Foundry artifact generation for Rust smart contracts

use crate::generators::metadata::BuildMetadata;
use anyhow::{Context, Result};
use fluentbase_types::hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Foundry artifact structure matching the expected JSON format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundryArtifact {
    pub abi: serde_json::Value,
    pub bytecode: Bytecode,
    #[serde(rename = "deployedBytecode")]
    pub deployed_bytecode: Bytecode,
    #[serde(rename = "methodIdentifiers")]
    pub method_identifiers: BTreeMap<String, String>,
    #[serde(rename = "rawMetadata")]
    pub raw_metadata: String,
    pub metadata: FoundryMetadata,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bytecode {
    pub object: String,
    #[serde(rename = "sourceMap")]
    pub source_map: String,
    #[serde(rename = "linkReferences")]
    pub link_references: BTreeMap<String, serde_json::Value>,
    #[serde(
        rename = "immutableReferences",
        skip_serializing_if = "Option::is_none"
    )]
    pub immutable_references: Option<BTreeMap<String, Vec<ImmutableReference>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImmutableReference {
    pub start: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundryMetadata {
    pub compiler: Compiler,
    pub language: String,
    pub output: Output,
    pub settings: Settings,
    pub sources: BTreeMap<String, SourceInfo>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compiler {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub abi: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devdoc: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub userdoc: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub remappings: Vec<String>,
    pub optimizer: Optimizer,
    pub metadata: MetadataSettings,
    #[serde(rename = "compilationTarget")]
    pub compilation_target: BTreeMap<String, String>,
    #[serde(rename = "evmVersion")]
    pub evm_version: String,
    pub libraries: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Optimizer {
    pub enabled: bool,
    pub runs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSettings {
    #[serde(rename = "bytecodeHash")]
    pub bytecode_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub keccak256: String,
    pub urls: Vec<String>,
    pub license: String,
}

/// Generate Foundry artifact from existing components
///
/// # Arguments
/// * `contract_name` - Clean contract name (e.g., "PowerCalculator")
/// * `abi` - Already generated ABI as JSON value
/// * `wasm_bytecode` - Compiled WASM bytecode
/// * `build_metadata` - Build metadata from metadata generator
/// * `interface_path` - Relative path to generated Solidity interface
///
/// # Returns
/// * `Result<FoundryArtifact>` - Complete Foundry-compatible artifact
pub fn generate_artifact(
    contract_name: &str,
    abi: &serde_json::Value,
    wasm_bytecode: &[u8],
    rwasm: &[u8],
    build_metadata: &BuildMetadata,
    interface_path: &str,
) -> Result<FoundryArtifact> {
    // Convert WASM bytecode to hex string
    let bytecode_hex = format!("0x{}", hex::encode(wasm_bytecode));
    let deployed_bytecode_hex = format!("0x{}", hex::encode(rwasm));

    // Create method identifiers from ABI (пока пустые)
    let method_identifiers = create_method_identifiers(abi)?;

    // Create Foundry metadata
    let foundry_metadata =
        create_foundry_metadata(contract_name, abi, build_metadata, interface_path)?;

    // Serialize metadata for rawMetadata field
    let raw_metadata =
        serde_json::to_string(&foundry_metadata).context("Failed to serialize metadata")?;

    // Create bytecode structures
    let bytecode = Bytecode {
        object: bytecode_hex.clone(),
        source_map: String::new(),
        link_references: BTreeMap::new(),
        immutable_references: None,
    };
    let deployed_bytecode = Bytecode {
        object: deployed_bytecode_hex.clone(),
        source_map: String::new(),
        link_references: BTreeMap::new(),
        immutable_references: None,
    };

    Ok(FoundryArtifact {
        abi: abi.clone(),
        bytecode,
        deployed_bytecode,
        method_identifiers,
        raw_metadata,
        metadata: foundry_metadata,
        id: 0,
    })
}

/// Create method identifiers from ABI (пока возвращает пустую мапу)
fn create_method_identifiers(_abi: &serde_json::Value) -> Result<BTreeMap<String, String>> {
    // TODO: Implement proper method identifier calculation
    Ok(BTreeMap::new())
}

/// Create Foundry metadata structure
fn create_foundry_metadata(
    contract_name: &str,
    abi: &serde_json::Value,
    build_metadata: &BuildMetadata,
    interface_path: &str,
) -> Result<FoundryMetadata> {
    // Create compilation target
    let mut compilation_target = BTreeMap::new();
    compilation_target.insert(interface_path.to_string(), contract_name.to_string());

    // Create sources info
    let mut sources = BTreeMap::new();
    sources.insert(
        interface_path.to_string(),
        SourceInfo {
            keccak256: "0x0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            urls: vec![],
            license: "MIT".to_string(),
        },
    );

    Ok(FoundryMetadata {
        compiler: Compiler {
            version: format!("rust-{}", build_metadata.environment.rustc_version),
        },
        language: "Rust".to_string(),
        output: Output {
            abi: abi.clone(),
            devdoc: None,
            userdoc: None,
        },
        settings: Settings {
            remappings: vec![],
            optimizer: Optimizer {
                enabled: build_metadata.build_config.wasm_opt,
                runs: 200,
            },
            metadata: MetadataSettings {
                bytecode_hash: "none".to_string(),
            },
            compilation_target,
            evm_version: "wasm".to_string(),
            libraries: BTreeMap::new(),
        },
        sources,
        version: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_artifact_basic() {
        let abi = serde_json::json!([]);
        let wasm_bytecode = b"test_bytecode";
    }
}

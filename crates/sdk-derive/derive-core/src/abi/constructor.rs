use super::types::rust_to_sol;
use crate::abi::{error::ABIError, parameter::Parameter};
use serde::{Deserialize, Serialize};
use syn::{FnArg, Pat, Signature};

pub const CONSTRUCTOR_ABI_TYPE: &str = "constructor";

/// Represents a constructor in the Solidity ABI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConstructorABI {
    /// Input parameters of the constructor
    pub inputs: Vec<Parameter>,

    /// State mutability (always nonpayable for constructors)
    #[serde(rename = "stateMutability")]
    pub state_mutability: String,

    /// Constructor type (always "constructor")
    #[serde(rename = "type")]
    pub abi_type: String,
}

impl ConstructorABI {
    pub fn from_signature(sig: &Signature) -> Result<Self, ABIError> {
        Ok(Self {
            inputs: Self::convert_inputs(&sig.inputs.iter().collect::<Vec<_>>())?,
            state_mutability: "nonpayable".to_string(),
            abi_type: CONSTRUCTOR_ABI_TYPE.to_string(),
        })
    }

    fn convert_inputs(inputs: &[&FnArg]) -> Result<Vec<Parameter>, ABIError> {
        inputs
            .iter()
            .enumerate()
            .filter_map(|(index, arg)| match arg {
                FnArg::Typed(pat_type) => {
                    let name = match &*pat_type.pat {
                        Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
                        _ => format!("_{index}"),
                    };
                    Some(
                        rust_to_sol(&pat_type.ty)
                            .map(|sol_type| Parameter::new(sol_type, name))
                            .map_err(ABIError::from),
                    )
                }
                FnArg::Receiver(_) => None,
            })
            .collect()
    }

    pub fn to_json(&self) -> Result<String, ABIError> {
        serde_json::to_string(self).map_err(|e| ABIError::Serialization(e.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_constructor_abi() {
        let sig: Signature = parse_quote! {
            fn constructor(admin: Address, initial_supply: U256)
        };

        let abi = ConstructorABI::from_signature(&sig).unwrap();

        assert_eq!(abi.inputs.len(), 2);
        assert_eq!(abi.state_mutability, "nonpayable");
        assert_eq!(abi.abi_type, "constructor");

        // Check inputs
        assert_eq!(abi.inputs[0].name, "admin");
        assert_eq!(abi.inputs[0].ty, "address");
        assert_eq!(abi.inputs[1].name, "initial_supply");
        assert_eq!(abi.inputs[1].ty, "uint256");
    }

    #[test]
    fn test_constructor_no_params() {
        let sig: Signature = parse_quote! {
            fn constructor()
        };

        let abi = ConstructorABI::from_signature(&sig).unwrap();

        assert_eq!(abi.inputs.len(), 0);
        assert_eq!(abi.state_mutability, "nonpayable");
        assert_eq!(abi.abi_type, "constructor");
    }

    #[test]
    fn test_constructor_json_serialization() {
        let sig: Signature = parse_quote! {
            fn constructor(owner: Address)
        };

        let abi = ConstructorABI::from_signature(&sig).unwrap();
        let json = abi.to_json().unwrap();

        // Check JSON structure
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "constructor");
        assert_eq!(value["stateMutability"], "nonpayable");
        assert!(value["inputs"].is_array());
    }
}

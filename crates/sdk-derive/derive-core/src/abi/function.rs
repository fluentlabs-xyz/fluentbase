use super::types::{rust_to_sol, ConversionError};
use crate::{
    abi::{error::ABIError, parameter::Parameter},
    artifacts::ArtifactsRegistry,
};
use convert_case::{Case, Casing};
use crypto_hashes::{digest::Digest, sha3::Keccak256};
use serde::{Deserialize, Serialize};
use syn::{FnArg, Pat, ReturnType, Signature, Type};

pub const FUNCTION_ABI_TYPE: &str = "function";
/// Represents a function in the Solidity ABI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionABI {
    /// Function name in camelCase
    pub name: String,

    /// Input parameters of the function
    pub inputs: Vec<Parameter>,

    /// Output parameters of the function
    pub outputs: Vec<Parameter>,

    /// State mutability (pure, view, nonpayable, payable)
    #[serde(rename = "stateMutability")]
    pub state_mutability: StateMutability,

    /// Function type (always "function" for regular functions)
    #[serde(rename = "type")]
    pub fn_type: String,
}

/// Represents state mutability in Solidity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StateMutability {
    /// Can't read state
    Pure,
    /// Can read but not modify state
    View,
    /// Can modify state
    NonPayable,
    /// Can receive ETH
    Payable,
}

impl From<ConversionError> for ABIError {
    fn from(err: ConversionError) -> Self {
        ABIError::TypeConversion(err.to_string())
    }
}

impl FunctionABI {
    pub fn from_signature(sig: &Signature) -> Result<Self, ABIError> {
        Ok(Self {
            name: sig.ident.to_string().to_case(Case::Camel),
            inputs: Self::convert_inputs(&sig.inputs.iter().collect::<Vec<_>>())?,
            outputs: Self::convert_outputs(&sig.output)?,
            state_mutability: StateMutability::NonPayable,
            fn_type: FUNCTION_ABI_TYPE.to_string(),
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

    fn convert_outputs(output: &ReturnType) -> Result<Vec<Parameter>, ABIError> {
        match output {
            ReturnType::Default => Ok(vec![]),
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple
                    .elems
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| {
                        rust_to_sol(ty)
                            .map(|sol_type| Parameter::new(sol_type, format!("_{i}")))
                            .map_err(ABIError::from)
                    })
                    .collect(),
                _ => Ok(vec![Parameter::new(
                    rust_to_sol(ty).map_err(ABIError::from)?,
                    "_0".to_string(),
                )]),
            },
        }
    }

    /// Returns canonical function signature for Solidity ABI
    /// Format: fnName(type1,type2,...)
    pub fn signature(&self) -> Result<String, ABIError> {
        let params = self
            .inputs
            .iter()
            .map(super::parameter::Parameter::get_canonical_type)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("{}({})", self.name, params.join(",")))
    }

    /// Calculates function selector (first 4 bytes of keccak256 hash)
    pub fn function_id(&self) -> Result<[u8; 4], ABIError> {
        let signature = self.signature()?;
        let mut hasher = Keccak256::new();
        hasher.update(signature.as_bytes());
        let result = hasher.finalize();

        let mut selector = [0u8; 4];
        selector.copy_from_slice(&result[..4]);
        Ok(selector)
    }

    // /// Returns the function name in snake_case (Rust style)
    // pub fn rust_name(&self) -> String {
    //     self.name.to_case(Case::Snake)
    // }

    // /// Returns the function name in camelCase (Solidity style)
    // pub fn solidity_name(&self) -> String {
    //     self.name.to_case(Case::Camel)
    // }

    pub fn to_json(&self) -> Result<String, ABIError> {
        serde_json::to_string(self).map_err(|e| ABIError::Serialization(e.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }

    pub fn from_json_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value)
    }

    pub fn load_structs(&self, registry: &ArtifactsRegistry) -> Result<Self, ABIError> {
        fn load_parameter(
            param: &Parameter,
            registry: &ArtifactsRegistry,
        ) -> Result<Parameter, ABIError> {
            if param.is_struct() {
                let struct_name =
                    param
                        .internal_type
                        .strip_prefix("struct ")
                        .ok_or(ABIError::Artifacts(format!(
                            "Invalid struct type: {:?}",
                            param.internal_type
                        )))?;
                let struct_path = format!("{}/{}.json", "structs", struct_name);
                let mut struct_param = registry.load::<Parameter>(&struct_path).map_err(|e| {
                    ABIError::Artifacts(format!("Failed to load struct {struct_name:?}: {e}"))
                })?;

                struct_param.name = param.name.clone();

                if let Some(components) = struct_param.components.as_mut() {
                    *components = components
                        .iter()
                        .map(|comp| load_parameter(comp, registry))
                        .collect::<Result<Vec<_>, _>>()?;
                }

                Ok(struct_param)
            } else {
                Ok(param.clone())
            }
        }

        let inputs = self
            .inputs
            .iter()
            .map(|param| load_parameter(param, registry))
            .collect::<Result<Vec<_>, _>>()?;

        let outputs = self
            .outputs
            .iter()
            .map(|param| load_parameter(param, registry))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(FunctionABI {
            inputs,
            outputs,
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_basic_function_abi() {
        let sig: Signature = parse_quote! {
            fn transfer(to: Address, amount: U256) -> bool
        };

        let abi = FunctionABI::from_signature(&sig).unwrap();

        assert_eq!(abi.name, "transfer");
        assert_eq!(abi.inputs.len(), 2);
        assert_eq!(abi.outputs.len(), 1);

        // Check inputs
        assert_eq!(abi.inputs[0].name, "to");
        assert_eq!(abi.inputs[0].ty, "address");
        assert_eq!(abi.inputs[1].name, "amount");
        assert_eq!(abi.inputs[1].ty, "uint256");

        // Check output
        assert_eq!(abi.outputs[0].name, "_0");
        assert_eq!(abi.outputs[0].ty, "bool");

        // Check signature and selector
        assert_eq!(abi.signature().unwrap(), "transfer(address,uint256)");
        assert_eq!(abi.function_id().unwrap(), [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn test_function_with_tuple_return() {
        let sig: Signature = parse_quote! {
            fn get_pair() -> (Address, U256)
        };

        let abi = FunctionABI::from_signature(&sig).unwrap();

        assert_eq!(abi.name, "getPair");
        assert_eq!(abi.inputs.len(), 0);
        assert_eq!(abi.outputs.len(), 2);

        // Check outputs
        assert_eq!(abi.outputs[0].name, "_0");
        assert_eq!(abi.outputs[0].ty, "address");
        assert_eq!(abi.outputs[1].name, "_1");
        assert_eq!(abi.outputs[1].ty, "uint256");
    }

    #[test]
    fn test_function_with_no_return() {
        let sig: Signature = parse_quote! {
            fn initialize(admin: Address)
        };

        let abi = FunctionABI::from_signature(&sig).unwrap();

        assert_eq!(abi.name, "initialize");
        assert_eq!(abi.inputs.len(), 1);
        assert_eq!(abi.outputs.len(), 0);

        // Check input
        assert_eq!(abi.inputs[0].name, "admin");
        assert_eq!(abi.inputs[0].ty, "address");
    }

    #[test]
    fn test_function_json_serialization() {
        let sig: Signature = parse_quote! {
            fn transfer(to: Address, amount: U256) -> bool
        };

        let abi = FunctionABI::from_signature(&sig).unwrap();
        let json = abi.to_json().unwrap();
        let deserialized = FunctionABI::from_json(&json).unwrap();

        assert_eq!(abi, deserialized);
    }
}

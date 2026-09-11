use super::types::{rust_to_sol, ConversionError};
use crate::abi::{error::ABIError, parameter::Parameter, structs::StructResolver};
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

    /// Builds the ABI with every struct parameter expanded into its components
    ///
    /// This is the representation both the router selector and the published artifacts are derived
    /// from; see [`crate::abi::structs`].
    pub fn from_signature_with(
        sig: &Signature,
        resolver: &StructResolver,
    ) -> Result<Self, ABIError> {
        let mut abi = Self::from_signature(sig)?;
        abi.resolve_structs(resolver)?;
        Ok(abi)
    }

    /// Expands the components of every struct parameter, if any parameter needs it
    pub fn resolve_structs(&mut self, resolver: &StructResolver) -> Result<(), ABIError> {
        if !self
            .inputs
            .iter()
            .chain(self.outputs.iter())
            .any(Parameter::has_unresolved_struct)
        {
            return Ok(());
        }

        let structs = resolver.structs().map_err(|error| {
            ABIError::StructResolution(format!(
                "{error}. Annotate the method with #[function_id(\"...\")] to pin its selector \
                 explicitly."
            ))
        })?;
        for parameter in self.inputs.iter_mut().chain(self.outputs.iter_mut()) {
            // Contract signatures live in the crate root, so they resolve from there
            parameter.resolve_structs(structs, "")?;
        }

        Ok(())
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

    /// Makes the entry hash to a pinned signature
    ///
    /// A `#[function_id("name(type,...)")]` attribute is the interface the router dispatches on,
    /// whatever the Rust parameter types derive to. The published entry has to say the same thing,
    /// otherwise callers encoding from the artifact never reach the method. The entry takes the
    /// pinned name, and every leaf parameter whose derived type differs takes the pinned type at
    /// its position. Tuples cannot be mapped onto a different pinned type, and a different number
    /// of parameters cannot be mapped at all; both are errors, and on an error the entry is left
    /// exactly as it was derived.
    pub fn retype_from_signature(&mut self, signature: &str) -> Result<(), ABIError> {
        let (name, params) = signature
            .strip_suffix(')')
            .and_then(|head| head.split_once('('))
            .ok_or_else(|| {
                ABIError::TypeConversion(format!(
                    "pinned signature `{signature}` is not of the form name(type,...)"
                ))
            })?;
        let pinned_types = split_top_level_types(params);

        if pinned_types.len() != self.inputs.len() {
            return Err(ABIError::TypeConversion(format!(
                "pinned signature `{signature}` has {} parameters, but `{}` takes {}",
                pinned_types.len(),
                self.name,
                self.inputs.len()
            )));
        }

        let mut inputs = self.inputs.clone();
        for (input, pinned_type) in inputs.iter_mut().zip(pinned_types) {
            let derived_type = input.get_canonical_type()?;
            if derived_type == pinned_type {
                continue;
            }
            if input.components.is_some() || pinned_type.starts_with('(') {
                return Err(ABIError::TypeConversion(format!(
                    "parameter `{}` derives to `{derived_type}`, but the pinned signature says \
                     `{pinned_type}`; a tuple parameter cannot be retyped, so the pinned \
                     signature has to spell out the same components",
                    input.name
                )));
            }
            input.internal_type = pinned_type.clone();
            input.ty = pinned_type;
        }

        self.inputs = inputs;
        self.name = name.to_string();

        Ok(())
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
}

/// Splits the parameter list of a canonical signature on the commas outside tuples
fn split_top_level_types(params: &str) -> Vec<String> {
    let mut types = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in params.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => types.push(core::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        types.push(current);
    }
    types
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    /// A pinned signature retypes the leaf parameters whose derived type differs
    #[test]
    fn test_pinned_signature_retypes_leaf_parameters() {
        let sig: Signature = parse_quote! {
            fn upgrade_to(target_address: Address, genesis_hash: B256, genesis_version: String, wasm_bytecode: Bytes)
        };
        let mut abi = FunctionABI::from_signature(&sig).unwrap();
        assert_eq!(
            abi.signature().unwrap(),
            "upgradeTo(address,bytes32,string,bytes)"
        );

        abi.retype_from_signature("upgradeTo(address,uint256,string,bytes)")
            .unwrap();

        assert_eq!(abi.inputs[1].name, "genesis_hash");
        assert_eq!(abi.inputs[1].ty, "uint256");
        assert_eq!(abi.inputs[1].internal_type, "uint256");
        assert_eq!(abi.inputs[0].ty, "address");
        assert_eq!(
            abi.signature().unwrap(),
            "upgradeTo(address,uint256,string,bytes)"
        );
        // keccak256("upgradeTo(address,uint256,string,bytes)")[..4]
        assert_eq!(abi.function_id().unwrap(), [0x28, 0x8f, 0xb3, 0xb8]);
    }

    /// The entry takes the pinned name, so the artifact describes what callers actually send
    #[test]
    fn test_pinned_signature_renames_the_entry() {
        let sig: Signature = parse_quote! {
            fn first_method(to: Address, amount: U256) -> bool
        };
        let mut abi = FunctionABI::from_signature(&sig).unwrap();

        abi.retype_from_signature("transfer(address,uint256)")
            .unwrap();

        assert_eq!(abi.name, "transfer");
        assert_eq!(abi.function_id().unwrap(), [0xa9, 0x05, 0x9c, 0xbb]);
    }

    /// Tuples keep their components: a pinned signature spelling them the same way is accepted
    #[test]
    fn test_pinned_signature_keeps_matching_tuples() {
        let sig: Signature = parse_quote! {
            fn set(config: (U256, bool), owner: Address)
        };
        let mut abi = FunctionABI::from_signature(&sig).unwrap();

        abi.retype_from_signature("set((uint256,bool),bytes32)")
            .unwrap();

        assert_eq!(abi.signature().unwrap(), "set((uint256,bool),bytes32)");
    }

    /// A tuple that disagrees with the pinned signature, or a different arity, cannot be mapped,
    /// and a failed mapping leaves the derived entry untouched
    #[test]
    fn test_pinned_signature_rejects_tuple_retypes_and_arity_changes() {
        let sig: Signature = parse_quote! {
            fn set(config: (U256, bool), owner: Address)
        };
        let mut abi = FunctionABI::from_signature(&sig).unwrap();
        let derived = abi.clone();

        assert!(abi
            .retype_from_signature("renamed((uint256,uint256),bytes32)")
            .is_err());
        assert!(abi.retype_from_signature("set(uint256,address)").is_err());
        assert!(abi.retype_from_signature("set((uint256,bool))").is_err());
        assert!(abi.retype_from_signature("set").is_err());
        assert_eq!(abi, derived);
    }

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
    fn test_byte_arrays_advertise_the_layout_the_codec_encodes() {
        // `[u8; N]` is encoded one word per element, so the selector must say `uint8[N]`.
        // Advertising `bytes32` would let canonical calldata select the route and then fail to
        // decode.
        let sig: Signature = parse_quote! {
            fn store(root: [u8; 32], tag: [u8; 4]) -> [u8; 32]
        };
        let abi = FunctionABI::from_signature(&sig).unwrap();
        assert_eq!(abi.inputs[0].ty, "uint8[32]");
        assert_eq!(abi.inputs[1].ty, "uint8[4]");
        assert_eq!(abi.outputs[0].ty, "uint8[32]");
        assert_eq!(abi.signature().unwrap(), "store(uint8[32],uint8[4])");

        // `bytesN` stays reachable through the types that carry the single-word codec.
        let sig: Signature = parse_quote! {
            fn store(root: FixedBytes<32>, tag: B32) -> B256
        };
        let abi = FunctionABI::from_signature(&sig).unwrap();
        assert_eq!(abi.signature().unwrap(), "store(bytes32,bytes4)");
        assert_eq!(abi.outputs[0].ty, "bytes32");
    }

    #[test]
    fn test_wide_fixed_bytes_aliases_have_no_signature() {
        // Solidity's fixed-bytes types stop at `bytes32`, and the codec writes `B512` as one
        // inline 64-byte blob that no Solidity type describes, so no selector can be derived.
        let sig: Signature = parse_quote! {
            fn verify(signature: B512) -> bool
        };
        assert!(FunctionABI::from_signature(&sig).is_err());
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

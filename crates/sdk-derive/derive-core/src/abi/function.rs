use super::{
    parameter::{FunctionParameter, SolType},
    ABIError,
};
use convert_case::{Case, Casing};
use crypto_hashes::{digest::Digest, sha3::Keccak256};
use serde::Serialize;
use syn::ImplItemFn;

/// Represents a function definition in Solidity ABI
#[derive(Debug, Clone, Serialize)]
pub struct FunctionABI {
    /// Function name
    pub name: String,
    /// Input parameters
    pub inputs: Vec<FunctionParameter>,
    /// Output parameters
    pub outputs: Vec<FunctionParameter>,
    /// State mutability (pure, view, nonpayable, payable)
    #[serde(rename = "stateMutability")]
    pub state_mutability: StateMutability,
    /// Function type (always "function" for regular functions)
    #[serde(rename = "type")]
    pub fn_type: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StateMutability {
    Pure,
    View,
    Nonpayable,
    Payable,
}

impl StateMutability {
    pub fn as_str(&self) -> &'static str {
        match self {
            StateMutability::Pure => "pure",
            StateMutability::View => "view",
            StateMutability::Nonpayable => "nonpayable",
            StateMutability::Payable => "payable",
        }
    }
}

impl FunctionABI {
    /// Creates a new FunctionABI from a Rust function
    pub fn from_impl_fn(function: &ImplItemFn) -> Result<Self, ABIError> {
        // Extract function name and convert to camel case
        let name = function.sig.ident.to_string().to_case(Case::Camel);

        // Parse input parameters
        let inputs = function
            .sig
            .inputs
            .iter()
            .filter_map(FunctionParameter::from_fn_arg)
            .collect::<Result<Vec<_>, _>>()?;

        // Parse output parameters
        let outputs = FunctionParameter::from_return_type(&function.sig.output)?;

        Ok(Self {
            name,
            inputs,
            outputs,
            state_mutability: StateMutability::Nonpayable,
            fn_type: "function".to_string(),
        })
    }

    /// Creates an FunctionABI from trait function
    pub fn from_trait_fn(function: &syn::TraitItemFn) -> Result<Self, ABIError> {
        // Extract function name and convert to camel case
        let name = function.sig.ident.to_string().to_case(Case::Camel);

        // Parse input parameters
        let inputs = function
            .sig
            .inputs
            .iter()
            .filter_map(FunctionParameter::from_fn_arg)
            .collect::<Result<Vec<_>, _>>()?;

        // Parse output parameters
        let outputs = FunctionParameter::from_return_type(&function.sig.output)?;

        Ok(Self {
            name,
            inputs,
            outputs,
            state_mutability: StateMutability::Nonpayable, // Default to nonpayable
            fn_type: "function".to_string(),
        })
    }

    /// Returns canonical function signature for Solidity ABI
    /// Format: name(type1,type2,...)
    pub fn signature(&self) -> String {
        let params = self
            .inputs
            .iter()
            .map(|param| param.sol_type.to_string())
            .collect::<Vec<_>>()
            .join(",");

        format!("{}({})", self.name, params)
    }

    pub fn rust_name(&self) -> String {
        self.name.to_case(Case::Snake)
    }

    /// Get complete function signature including parameter names and return types
    /// Format: function name(type1 name1, type2 name2) returns (type1 name1, type2 name2)
    pub fn full_signature(&self) -> String {
        // Format input parameters with names
        let inputs = self
            .inputs
            .iter()
            .map(|param| {
                let type_str = param.sol_type.to_string();
                match &param.name {
                    Some(name) => format!("{} {}", type_str, name),
                    None => type_str,
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Format output parameters with names
        let outputs = self
            .outputs
            .iter()
            .map(|param| {
                let type_str = param.sol_type.to_string();
                match &param.name {
                    Some(name) => format!("{} {}", type_str, name),
                    None => type_str,
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Construct full signature with mutability
        let mut signature = format!(
            "function {}({}) {}",
            self.name,
            inputs,
            self.state_mutability.as_str()
        );

        // Add return types if present
        if !self.outputs.is_empty() {
            signature.push_str(&format!(" returns ({})", outputs));
        }

        signature
    }

    /// Calculate function selector (first 4 bytes of keccak256 hash)
    pub fn selector(&self) -> [u8; 4] {
        let mut hasher = Keccak256::new();
        hasher.update(self.signature().as_bytes());
        let result = hasher.finalize();

        let mut selector = [0u8; 4];
        selector.copy_from_slice(&result[..4]);
        selector
    }

    /// Generates Solidity interface representation of the function
    pub fn to_solidity_interface(&self) -> String {
        // Format input parameters with proper data location
        let inputs = self
            .inputs
            .iter()
            .map(|param| {
                let type_str = param.sol_type.to_string();
                let name = param.name.as_deref().unwrap_or("_");

                // Add memory location for complex types (required in external functions)
                let location = if needs_data_location(&param.sol_type) {
                    " memory" // Could be calldata for better gas optimization
                } else {
                    ""
                };

                format!("{}{} {}", type_str, location, name)
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Format output parameters (names are optional in interfaces)
        let outputs = self
            .outputs
            .iter()
            .map(|param| {
                let type_str = param.sol_type.to_string();
                let location = if needs_data_location(&param.sol_type) {
                    " memory"
                } else {
                    ""
                };

                // In interfaces, return parameter names are optional
                format!("{}{}", type_str, location)
            })
            .collect::<Vec<_>>()
            .join(", ");

        let mut signature = format!("    function {}({}) external", self.name, inputs);

        // Add state mutability if it's view or pure
        match self.state_mutability {
            StateMutability::View => signature.push_str(" view"),
            StateMutability::Pure => signature.push_str(" pure"),
            StateMutability::Payable => signature.push_str(" payable"),
            StateMutability::Nonpayable => {} // default, no need to specify
        }

        if !self.outputs.is_empty() {
            signature.push_str(&format!(" returns ({})", outputs));
        }

        signature.push(';');
        signature
    }
}

/// Check if the type requires a data location specifier
fn needs_data_location(sol_type: &SolType) -> bool {
    matches!(
        sol_type,
        SolType::String
            | SolType::Bytes
            | SolType::Array(_)
            | SolType::FixedArray(_, _)
            | SolType::Tuple(_)
            | SolType::Struct { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use syn::parse_quote;

    #[test]
    fn test_simple_function() {
        let function: ImplItemFn = parse_quote! {
            fn add(a: u256, b: u256) -> u256 {
                a + b
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        let json = serde_json::to_value(&abi).unwrap();

        assert_eq!(
            json,
            json!({
                "name": "add",
                "type": "function",
                "inputs": [
                    {
                        "name": "a",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "b",
                        "type": "uint256",
                        "internalType": "uint256"
                    }
                ],
                "outputs": [
                    {
                        "name": "_0",
                        "type": "uint256",
                        "internalType": "uint256"
                    }
                ],
                "stateMutability": "nonpayable"
            })
        );
    }

    #[test]
    fn test_tuple_output() {
        let function: ImplItemFn = parse_quote! {
            fn get_pair() -> (u256, bool) {
                (0.into(), true)
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        let json = serde_json::to_value(&abi).unwrap();

        assert_eq!(
            json,
            json!({
                "name": "getPair",
                "type": "function",
                "inputs": [],
                "outputs": [
                    {
                        "name": "_0",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "_1",
                        "type": "bool",
                        "internalType": "bool"
                    }
                ],
                "stateMutability": "nonpayable"
            })
        );
    }

    #[test]
    fn test_function_signatures() {
        let function: ImplItemFn = parse_quote! {
            fn transfer(recipient: Address, amount: u256) -> bool {
                true
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();

        // Test basic signature
        assert_eq!(abi.signature(), "transfer(address,uint256)");

        // Test selector calculation
        let selector = abi.selector();
        assert_eq!(hex::encode(selector), "a9059cbb");
    }

    #[test]
    fn test_full_signature() {
        // Test simple function
        let function: ImplItemFn = parse_quote! {
            fn transfer(recipient: Address, amount: u256) -> bool {
                true
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        assert_eq!(
            abi.full_signature(),
            "function transfer(address recipient, uint256 amount) nonpayable returns (bool _0)"
        );

        // Test function with complex types
        let function: ImplItemFn = parse_quote! {
            fn complex_operation(
                addresses: Vec<Address>,
                amounts: (u256, u256),
                data: FixedBytes<32>
            ) -> (bool, u256) {
                (true, 0.into())
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        assert_eq!(
            abi.full_signature(),
            "function complexOperation(address[] addresses, (uint256,uint256) amounts, bytes32 data) nonpayable returns (bool _0, uint256 _1)"
        );

        // Test function without return value
        let function: ImplItemFn = parse_quote! {
            fn do_something(value: bool) {
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        assert_eq!(
            abi.full_signature(),
            "function doSomething(bool value) nonpayable"
        );

        // Test function with tuple returns
        let function: ImplItemFn = parse_quote! {
            fn get_pair() -> (Address, bool) {
                (Address::default(), true)
            }
        };

        let abi = FunctionABI::from_impl_fn(&function).unwrap();
        assert_eq!(
            abi.full_signature(),
            "function getPair() nonpayable returns (address _0, bool _1)"
        );
    }

    #[test]
    fn test_solidity_interface_generation() {
        use super::*;
        use syn::parse_quote;

        fn create_test_function(function: ImplItemFn) -> FunctionABI {
            FunctionABI::from_impl_fn(&function).unwrap()
        }

        // Test basic nonpayable function
        let function: ImplItemFn = parse_quote! {
            fn transfer(recipient: Address, amount: u256) -> bool {
                true
            }
        };
        let abi = create_test_function(function);
        assert_eq!(
            abi.to_solidity_interface(),
            "    function transfer(address recipient, uint256 amount) external returns (bool);"
        );

        // Test memory types
        let function: ImplItemFn = parse_quote! {
            fn handle_data(recipients: Vec<Address>, memo: String) -> Vec<bool> {
                vec![]
            }
        };
        let abi = create_test_function(function);
        assert_eq!(
            abi.to_solidity_interface(),
            "    function handleData(address[] memory recipients, string memory memo) external returns (bool[] memory);"
        );

        // Test view function
        let mut abi = create_test_function(parse_quote! {
            fn get_value() -> u256 { 0.into() }
        });
        abi.state_mutability = StateMutability::View;
        assert_eq!(
            abi.to_solidity_interface(),
            "    function getValue() external view returns (uint256);"
        );

        // Test pure function
        let mut abi = create_test_function(parse_quote! {
            fn pure_calc(x: u256) -> u256 { x }
        });
        abi.state_mutability = StateMutability::Pure;
        assert_eq!(
            abi.to_solidity_interface(),
            "    function pureCalc(uint256 x) external pure returns (uint256);"
        );

        // Test payable function
        let mut abi = create_test_function(parse_quote! {
            fn deposit(note: String) {}
        });
        abi.state_mutability = StateMutability::Payable;
        assert_eq!(
            abi.to_solidity_interface(),
            "    function deposit(string memory note) external payable;"
        );

        // Test tuple types
        let function: ImplItemFn = parse_quote! {
            fn tuple_data(data: (u256, bool)) -> (Address, String) {
                (Address::default(), String::new())
            }
        };
        let abi = create_test_function(function);
        assert_eq!(
            abi.to_solidity_interface(),
            "    function tupleData((uint256,bool) memory data) external returns (address, string memory);"
        );
    }
}

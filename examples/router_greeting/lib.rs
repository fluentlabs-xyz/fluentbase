#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;
use alloc::string::String;
use fluentbase_sdk::{alloc_slice, basic_entrypoint, router, signature, SharedAPI};

#[derive(Default)]
struct GREETING;

// There are only solidity mode supported for now
#[router(mode = "solidity")]
impl GREETING {
    // here should be a valid Solidity function signature
    #[signature("function greet(string msg) external returns (string)")]
    pub fn greet(&self, msg: String) -> String {
        msg
    }
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }

    fn main<SDK: SharedAPI>(&self) {
        // get size of the input and allocate memory for input
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        // copy input to the allocated memory
        SDK::read(input, 0);

        let output = alloc_slice(1024 as usize);
        let output_size: usize = self.route(input, output);
        SDK::write(&output[..output_size]);
    }
}

basic_entrypoint!(GREETING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;
    extern crate hex;
    // use alloy_sol_types::{sol_data::Uint, SolType};

    #[test]
    fn test_contract_method_works() {
        let input = "ead710c40000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c48656c6c6f20776f726c64210000000000000000000000000000000000000000";
        let input = hex::decode(input).expect("Failed to decode hex string");

        let msg = match greetCall::abi_decode(&input, true) {
            Ok(decoded) => decoded.msg,
            Err(e) => {
                {
                    panic!("Failed to decode input {:?} {:?}", "msg", e,);
                };
            }
        };

        LowLevelSDK::with_test_input(input.clone());

        let greeting = GREETING::default();
        greeting.deploy::<LowLevelSDK>();
        greeting.main::<LowLevelSDK>();

        let test_output = LowLevelSDK::get_test_output();
        let res = greetCall::abi_decode_returns(&test_output, false).unwrap();

        assert_eq!(msg, res._0);
    }
}

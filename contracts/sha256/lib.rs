#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, ContextReader, ExitCode, SharedAPI};

fn sha256_with_sdk<SDK: SharedAPI>(_: &SDK, data: &[u8]) -> fluentbase_sdk::B256 {
    SDK::sha256(data)
}

/// Main entry point for the sha256 wrapper contract.
/// This contract wraps the sha256 precompile (EIP-210) which computes the SHA-256 hash of a given input.
///
/// Input:
/// - A byte array of arbitrary length
///
/// Output:
/// - A 32-byte array representing the SHA-256 hash of the input
///
pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);

    let gas_used = estimate_gas(input.len());
    if gas_used > gas_limit {
        sdk.native_exit(ExitCode::OutOfFuel);
    }
    let result = sha256_with_sdk(&sdk, &input);
    sdk.sync_evm_gas(gas_used, 0);
    sdk.write(result.0.as_ref());
}

/// Gas estimation for SHA-256 (based on EVM gas model)
/// - Base cost: 60 gas
/// - Per word (32 bytes): 12 gas
#[inline(always)]
fn estimate_gas(input_len: usize) -> u64 {
    let words = (input_len + 31) / 32;
    60 + (words as u64 * 12)
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_sdk_testing::HostTestingContext;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(output, expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    #[test]
    fn test_hello_world_works() {
        exec_evm_precompile(
            "Hello, World".as_bytes(),
            &hex!("03675ac53ff9cd1535ccc7dfcdfa2c458c5218371f418dc136f2d19ac1fbe8a5"),
            72,
        );
    }
}

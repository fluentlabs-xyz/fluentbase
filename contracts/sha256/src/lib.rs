#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{crypto::crypto_sha256, system_entrypoint, ExitCode, SystemAPI};

/// Main entry point for the sha256 wrapper contract.
/// This contract wraps the sha256 precompile (EIP-210) which computes the SHA-256 hash of a given input.
///
/// Input:
/// - A byte array of arbitrary length
///
/// Output:
/// - A 32-byte array representing the SHA-256 hash of the input
///
pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_length = sdk.input_size();
    let gas_used = estimate_gas(input_length as usize);
    sdk.sync_evm_gas(gas_used)?;
    let input = sdk.bytes_input();
    let result = crypto_sha256(input);
    sdk.write(result);
    Ok(())
}

/// Gas estimation for SHA-256 (based on an EVM gas model)
/// - Base cost: 60 gas
/// - Per word (32 bytes): 12 gas
#[inline(always)]
fn estimate_gas(input_len: usize) -> u64 {
    let words = input_len.div_ceil(32);
    60 + (words as u64 * 12)
}

system_entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Bytes, ContractContextV1, SharedAPI, FUEL_DENOM_RATE};
    use fluentbase_testing::TestingContextImpl;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let mut sdk = TestingContextImpl::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        main_entry(&mut sdk).unwrap();
        let output = sdk.take_output();
        assert_eq!(&output, expected);
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

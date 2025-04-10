#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, ContractContextReader, ExitCode, SharedAPI};
use revm_precompile::{
    calc_linear_cost_u32,
    identity::{IDENTITY_BASE, IDENTITY_PER_WORD},
};

pub fn main(mut sdk: impl SharedAPI) {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    // fail fast if we don't have enough fuel for the call
    let gas_used = calc_linear_cost_u32(input_length as usize, IDENTITY_BASE, IDENTITY_PER_WORD);
    if gas_used > gas_limit {
        sdk.exit(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_limit - gas_used, 0);
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    // write an identical output
    sdk.write(input);
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{testing::TestingContext, Bytes, ContractContextV1};

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000_000;
        let sdk = TestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                // gas_limit,
                ..Default::default()
            });
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(output, expected);
        let (gas_remaining, gas_refunded) = sdk.synced_gas();
        assert_eq!(gas_limit - gas_remaining, expected_gas);
        assert_eq!(gas_refunded, 0);
    }

    #[test]
    fn test_hello_world_works() {
        exec_evm_precompile("Hello, World".as_bytes(), "Hello, World".as_bytes(), 18);
    }
}

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, ContextReader, ExitCode, SharedAPI};
use revm_precompile::{
    calc_linear_cost_u32,
    identity::{IDENTITY_BASE, IDENTITY_PER_WORD},
};

pub fn main_entry(mut sdk: impl SharedAPI) {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    // fail fast if we don't have enough fuel for the call
    let gas_used = calc_linear_cost_u32(input_length as usize, IDENTITY_BASE, IDENTITY_PER_WORD);
    if gas_used > gas_limit {
        sdk.native_exit(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_used);
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    // write an identical output
    sdk.write(input);
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;

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
        exec_evm_precompile("Hello, World".as_bytes(), "Hello, World".as_bytes(), 18);
    }
}

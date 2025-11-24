#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{system_runtime_entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};
use revm_precompile::{
    calc_linear_cost_u32,
    identity::{IDENTITY_BASE, IDENTITY_PER_WORD},
};

pub fn main_entry(sdk: &mut impl SharedAPI) -> (Bytes, ExitCode) {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    // fail fast if we don't have enough fuel for the call
    let gas_used = calc_linear_cost_u32(input_length as usize, IDENTITY_BASE, IDENTITY_PER_WORD);
    if gas_used > gas_limit {
        return (Bytes::new(), ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_used);
    // write an identical output
    (sdk.bytes_input(), ExitCode::Ok)
}

system_runtime_entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let mut sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        let (output, exit_code) = main_entry(&mut sdk);
        assert_eq!(exit_code, ExitCode::Ok);
        assert_eq!(output.as_ref(), expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    #[test]
    fn test_hello_world_works() {
        exec_evm_precompile("Hello, World".as_bytes(), "Hello, World".as_bytes(), 18);
    }
}

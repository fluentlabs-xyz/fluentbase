#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{system_entrypoint, ContextReader, ExitCode, SystemAPI};
use revm_precompile::PrecompileError;

pub fn main_entry(sdk: &mut impl SystemAPI) -> Result<(), ExitCode> {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input = sdk.bytes_input();
    // call ripemd160 function
    let result =
        revm_precompile::hash::ripemd160_run(&input, gas_limit).map_err(|err| match err {
            PrecompileError::OutOfGas => ExitCode::OutOfFuel,
            _ => ExitCode::PrecompileError,
        })?;
    sdk.sync_evm_gas(result.gas_used)?;
    // write output
    sdk.write(result.bytes);
    Ok(())
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
            &hex!("0000000000000000000000006782893f9a818abc3da35d745a803d72a660c9f5"),
            720,
        );
    }
}

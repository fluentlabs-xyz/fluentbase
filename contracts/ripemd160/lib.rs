#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call ripemd160 function
    let result = revm_precompile::hash::ripemd160_run(&input, gas_limit)
        .unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));
    sdk.sync_evm_gas(result.gas_used, 0);
    // write output
    sdk.write(result.bytes.as_ref());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, FUEL_DENOM_RATE};
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
        exec_evm_precompile(
            "Hello, World".as_bytes(),
            &hex!("0000000000000000000000006782893f9a818abc3da35d745a803d72a660c9f5"),
            720,
        );
    }
}

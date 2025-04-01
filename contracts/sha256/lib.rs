#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    func_entrypoint,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
};

pub fn main(mut sdk: impl SharedAPI) {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call sha256 function
    let result = revm_precompile::hash::sha256_run(&input, gas_limit)
        .unwrap_or_else(|err| sdk.exit(ExitCode::from(err).into_i32()));
    sdk.sync_evm_gas(gas_limit - result.gas_used, 0);
    // write output
    sdk.write(result.bytes.as_ref());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, testing::TestingContext, ContractContextV1};

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let sdk = TestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
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
        exec_evm_precompile(
            "Hello, World".as_bytes(),
            &hex!("03675ac53ff9cd1535ccc7dfcdfa2c458c5218371f418dc136f2d19ac1fbe8a5"),
            72,
        );
    }
}

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call identity function
    let result = revm_precompile::secp256k1::ec_recover_run(&input, gas_limit)
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
    fn call_ecrecover_unrecoverable_key() {
        exec_evm_precompile(
            &hex!("a8b53bdf3306a35a7103ab5504a0c9b492295564b6202b1942a84ef300107281000000000000000000000000000000000000000000000000000000000000001b307835653165303366353363653138623737326363623030393366663731663366353366356337356237346463623331613835616138623838393262346538621122334455667788991011121314151617181920212223242526272829303132"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn valid_key() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!("000000000000000000000000a94f5374fce5edbc8e2a8697c15331677e6ebf0b"),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_1() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c100000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_2() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000001000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }

    #[test]
    fn invalid_high_v_bits_3() {
        exec_evm_precompile(
            &hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000001000000000000000000000011c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549"),
            &hex!(""),
            3000,
        );
    }
}

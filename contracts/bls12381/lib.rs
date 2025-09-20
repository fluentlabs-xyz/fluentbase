#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI,
    PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let bytecode_address = sdk.context().contract_bytecode_address();
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // call precompiled function
    let precompile_func = match bytecode_address {
        PRECOMPILE_BLS12_381_G1_ADD => revm_precompile::bls12_381::g1_add::g1_add,
        PRECOMPILE_BLS12_381_G1_MSM => revm_precompile::bls12_381::g1_msm::g1_msm,
        PRECOMPILE_BLS12_381_G2_ADD => revm_precompile::bls12_381::g2_add::g2_add,
        PRECOMPILE_BLS12_381_G2_MSM => revm_precompile::bls12_381::g2_msm::g2_msm,
        PRECOMPILE_BLS12_381_PAIRING => revm_precompile::bls12_381::pairing::pairing,
        PRECOMPILE_BLS12_381_MAP_G1 => revm_precompile::bls12_381::map_fp_to_g1::map_fp_to_g1,
        PRECOMPILE_BLS12_381_MAP_G2 => revm_precompile::bls12_381::map_fp2_to_g2::map_fp2_to_g2,
        _ => unreachable!("bls12381: unsupported contract address"),
    };
    let result = precompile_func(&input, gas_limit)
        .unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));
    sdk.sync_evm_gas(result.gas_used, 0);
    // write output
    sdk.write(result.bytes.as_ref());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;

    fn exec_evm_precompile(address: Address, inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
        let sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                address,
                bytecode_address: address,
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
    fn bls_g1add_2_g1_3_g1_5_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_ADD,
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d280000000000000000000000000000000009ece308f9d1f0131765212deca99697b112d61f9be9a5f1f3780a51335b3ff981747a0b2ca2179b96d2c0c9024e522400000000000000000000000000000000032b80d3a6f5b09f8a84623389c5f80ca69a0cddabc3097f9d9c27310fd43be6e745256c634af45ca3473b0590ae30d1"),
            &hex!("0000000000000000000000000000000010e7791fb972fe014159aa33a98622da3cdc98ff707965e536d8636b5fcc5ac7a91a8c46e59a00dca575af0f18fb13dc0000000000000000000000000000000016ba437edcc6551e30c10512367494bfb6b01cc6681e8a4c3cd2501832ab5c4abc40b4578b85cbaffbf0bcd70d67c6e2"),
            375,
        );
    }
}

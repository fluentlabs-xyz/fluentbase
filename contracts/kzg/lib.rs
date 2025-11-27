#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{system_entrypoint, Bytes, ContextReader, ExitCode, SharedAPI};

pub fn main_entry(sdk: &mut impl SharedAPI) -> Result<Bytes, ExitCode> {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input = sdk.bytes_input().clone();
    // call blake2 function
    let result = revm_precompile::kzg_point_evaluation::run(&input, gas_limit)
        .map_err(|_| ExitCode::PrecompileError)?;
    sdk.sync_evm_gas(result.gas_used)?;
    // write output
    Ok(result.bytes)
}

system_entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;
    use revm_precompile::kzg_point_evaluation::VERSIONED_HASH_VERSION_KZG;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 10_000_000;
        let mut sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        let output = main_entry(&mut sdk).unwrap();
        assert_eq!(output.as_ref(), expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    #[test]
    fn test_kzg_proof_case_correct_proof() {
        // test data from: https://github.com/ethereum/c-kzg-4844/blob/main/tests/verify_kzg_proof/kzg-mainnet/verify_kzg_proof_case_correct_proof_31ebd010e6098750/data.yaml
        let commitment = hex!("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7").to_vec();
        let mut versioned_hash =
            hex!("f7e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b").to_vec();
        versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
        let z = hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000").to_vec();
        let y = hex!("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9").to_vec();
        let proof = hex!("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c").to_vec();
        let input = [versioned_hash, z, y, commitment, proof].concat();
        let expected_output = &hex!("000000000000000000000000000000000000000000000000000000000000100073eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");
        let gas = 50000;
        exec_evm_precompile(&input, expected_output, gas);
    }
}

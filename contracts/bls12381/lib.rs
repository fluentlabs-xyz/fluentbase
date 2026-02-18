#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;

use fluentbase_sdk::{
    system_entrypoint, ContextReader, ExitCode, SharedAPI, PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
};
use revm_precompile::{
    bls12_381::{
        g1_add::g1_add, g1_msm::g1_msm, g2_add::g2_add, g2_msm::g2_msm,
        map_fp2_to_g2::map_fp2_to_g2, map_fp_to_g1::map_fp_to_g1, pairing::pairing,
    },
    bls12_381_const::{
        DISCOUNT_TABLE_G1_MSM, DISCOUNT_TABLE_G2_MSM, G1_ADD_BASE_GAS_FEE, G1_MSM_BASE_GAS_FEE,
        G1_MSM_INPUT_LENGTH, G2_ADD_BASE_GAS_FEE, G2_MSM_BASE_GAS_FEE, G2_MSM_INPUT_LENGTH,
        MAP_FP2_TO_G2_BASE_GAS_FEE, MAP_FP_TO_G1_BASE_GAS_FEE, PADDED_FP2_LENGTH, PADDED_FP_LENGTH,
        PAIRING_INPUT_LENGTH, PAIRING_MULTIPLIER_BASE, PAIRING_OFFSET_BASE,
    },
    bls12_381_utils::msm_required_gas,
    PrecompileError, PrecompileOutput,
};

fn g1_add_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    if G1_ADD_BASE_GAS_FEE > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input = sdk.input();
    g1_add(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn g2_add_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    if G2_ADD_BASE_GAS_FEE > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input = sdk.input();
    g2_add(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn g1_msm_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_size = sdk.input_size() as usize;
    if input_size == 0 || !input_size.is_multiple_of(G1_MSM_INPUT_LENGTH) {
        return Err(ExitCode::PrecompileError);
    }
    let k = input_size / G1_MSM_INPUT_LENGTH;
    let required_gas = msm_required_gas(k, &DISCOUNT_TABLE_G1_MSM, G1_MSM_BASE_GAS_FEE);
    if required_gas > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input = sdk.input();
    g1_msm(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn g2_msm_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_size = sdk.input_size() as usize;
    if input_size == 0 || !input_size.is_multiple_of(G2_MSM_INPUT_LENGTH) {
        return Err(ExitCode::PrecompileError);
    }
    let k = input_size / G2_MSM_INPUT_LENGTH;
    let required_gas = msm_required_gas(k, &DISCOUNT_TABLE_G2_MSM, G2_MSM_BASE_GAS_FEE);
    if required_gas > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input = sdk.input();
    g2_msm(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn pairing_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    let input_size = sdk.input_size() as usize;
    if input_size == 0 || !input_size.is_multiple_of(PAIRING_INPUT_LENGTH) {
        return Err(ExitCode::PrecompileError);
    }
    let k = input_size / PAIRING_INPUT_LENGTH;
    let required_gas: u64 = PAIRING_MULTIPLIER_BASE * k as u64 + PAIRING_OFFSET_BASE;
    if required_gas > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input = sdk.input();
    pairing(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn map_fp_to_g1_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    if MAP_FP_TO_G1_BASE_GAS_FEE > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input_size = sdk.input_size() as usize;
    if input_size != PADDED_FP_LENGTH {
        return Err(ExitCode::PrecompileError);
    }
    let input = sdk.input();
    map_fp_to_g1(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

fn map_fp2_to_g2_checked<SDK: SharedAPI>(sdk: &mut SDK) -> Result<PrecompileOutput, ExitCode> {
    let gas_limit = sdk.context().contract_gas_limit();
    if MAP_FP2_TO_G2_BASE_GAS_FEE > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    let input_size = sdk.input_size() as usize;
    if input_size != PADDED_FP2_LENGTH {
        return Err(ExitCode::PrecompileError);
    }
    let input = sdk.input();
    map_fp2_to_g2(input, gas_limit).map_err(|err| match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        _ => ExitCode::PrecompileError,
    })
}

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let bytecode_address = sdk.context().contract_bytecode_address();
    // dispatch to SDK-backed implementation (w/ pre-gas/input checks)
    let result = match bytecode_address {
        PRECOMPILE_BLS12_381_G1_ADD => g1_add_checked(sdk),
        PRECOMPILE_BLS12_381_G2_ADD => g2_add_checked(sdk),
        PRECOMPILE_BLS12_381_G1_MSM => g1_msm_checked(sdk),
        PRECOMPILE_BLS12_381_G2_MSM => g2_msm_checked(sdk),
        PRECOMPILE_BLS12_381_PAIRING => pairing_checked(sdk),
        PRECOMPILE_BLS12_381_MAP_G1 => map_fp_to_g1_checked(sdk),
        PRECOMPILE_BLS12_381_MAP_G2 => map_fp2_to_g2_checked(sdk),
        _ => unreachable!("bls12381: unsupported contract address"),
    }?;
    sdk.sync_evm_gas(result.gas_used)?;
    sdk.write(result.bytes);
    Ok(())
}

system_entrypoint!(main_entry);

/**
 * The following are the tests for the BLS12-381 precompile contract.
 *
 * Note: The tests cases are taken from the: https://eips.ethereum.org/assets/eip-2537/test-vectors
 */
#[cfg(test)]
mod tests {
    use crate::{
        main_entry, PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM,
        PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1,
        PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
    };
    use fluentbase_sdk::{hex, Address, ContractContextV1, SharedAPI, FUEL_DENOM_RATE};
    use fluentbase_testing::TestingContextImpl;
    use serde::Deserialize;

    /// Must match the JSON keys exactly.
    #[derive(Clone, Debug, Deserialize)]
    struct BlsTestVector {
        #[serde(rename = "Input")]
        input: String,
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Expected")]
        expected: Option<String>,
        #[serde(rename = "ExpectedError")]
        expected_error: Option<String>,
        #[serde(rename = "Gas")]
        gas: Option<u64>,
        #[serde(rename = "NoBenchmark")]
        _no_benchmark: Option<bool>,
    }

    fn decode_hex(s: &str) -> Vec<u8> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        hex::decode(s).expect("invalid hex in test vector")
    }

    fn exec_evm_precompile(address: Address, bls_test_vector: BlsTestVector) {
        let input = decode_hex(&bls_test_vector.input);
        let expected = bls_test_vector.expected.map(|v| decode_hex(&v));
        let gas_limit = 2_000_000;
        let mut sdk = TestingContextImpl::default()
            .with_input(input)
            .with_contract_context(ContractContextV1 {
                address,
                bytecode_address: address,
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        if let Some(expected) = expected {
            main_entry(&mut sdk).unwrap();
            let output = sdk.take_output();
            assert_eq!(
                output.as_ref(),
                expected,
                "unexpected output for precompile at {address:?}"
            );
        } else {
            let _err = main_entry(&mut sdk).unwrap_err();
            let _expected_error = bls_test_vector.expected_error.unwrap();
            // TODO(dmitry123): Check error, but now we always return the same PrecompileError
        }
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        if let Some(expected_gas) = bls_test_vector.gas {
            assert_eq!(
                gas_limit - gas_remaining,
                expected_gas,
                "unexpected gas for precompile at {address:?}"
            );
        }
    }

    /// Run all vectors from a single JSON file for a given precompile address.
    ///
    /// `file_stem` is the filename without `.json`, e.g. "add_G1_bls".
    fn run_bls_file(json: &str, address: Address) {
        let vectors: Vec<BlsTestVector> =
            serde_json::from_str(json).expect("failed to parse BLS test JSON");
        for v in vectors {
            print!("+ running test case: {}... ", v.name);
            exec_evm_precompile(address, v);
            println!("DONE");
        }
    }

    macro_rules! bls_file_test {
        ($fn_name:ident, $file_stem:literal, $addr:expr) => {
            #[test]
            fn $fn_name() {
                run_bls_file(
                    include_str!(concat!("testcases/", $file_stem, ".json")),
                    $addr,
                );
            }
        };
    }

    // Mapping notes:
    // - add_G1 / add_G2 -> G1_ADD / G2_ADD
    // - msm_G1 / mul_G1 -> G1_MSM precompile (mul is MSM with len=1)
    // - msm_G2 / mul_G2 -> G2_MSM precompile
    // - pairing_check_*  -> PAIRING
    // - map_fp_to_G1_*   -> MAP_G1
    // - map_fp2_to_G2_*  -> MAP_G2

    bls_file_test!(bls_add_g1, "add_G1_bls", PRECOMPILE_BLS12_381_G1_ADD);
    bls_file_test!(bls_add_g2, "add_G2_bls", PRECOMPILE_BLS12_381_G2_ADD);

    bls_file_test!(
        bls_fail_add_g1,
        "fail-add_G1_bls",
        PRECOMPILE_BLS12_381_G1_ADD
    );
    bls_file_test!(
        bls_fail_add_g2,
        "fail-add_G2_bls",
        PRECOMPILE_BLS12_381_G2_ADD
    );

    bls_file_test!(
        bls_fail_map_fp2_to_g2,
        "fail-map_fp2_to_G2_bls",
        PRECOMPILE_BLS12_381_MAP_G2
    );
    bls_file_test!(
        bls_fail_map_fp_to_g1,
        "fail-map_fp_to_G1_bls",
        PRECOMPILE_BLS12_381_MAP_G1
    );

    bls_file_test!(
        bls_fail_msm_g1,
        "fail-msm_G1_bls",
        PRECOMPILE_BLS12_381_G1_MSM
    );
    bls_file_test!(
        bls_fail_msm_g2,
        "fail-msm_G2_bls",
        PRECOMPILE_BLS12_381_G2_MSM
    );

    // mul_* go through MSM precompiles too.
    bls_file_test!(
        bls_fail_mul_g1,
        "fail-mul_G1_bls",
        PRECOMPILE_BLS12_381_G1_MSM
    );
    bls_file_test!(
        bls_fail_mul_g2,
        "fail-mul_G2_bls",
        PRECOMPILE_BLS12_381_G2_MSM
    );

    bls_file_test!(
        bls_fail_pairing_check,
        "fail-pairing_check_bls",
        PRECOMPILE_BLS12_381_PAIRING
    );

    bls_file_test!(
        bls_map_fp2_to_g2,
        "map_fp2_to_G2_bls",
        PRECOMPILE_BLS12_381_MAP_G2
    );
    bls_file_test!(
        bls_map_fp_to_g1,
        "map_fp_to_G1_bls",
        PRECOMPILE_BLS12_381_MAP_G1
    );

    bls_file_test!(bls_msm_g1, "msm_G1_bls", PRECOMPILE_BLS12_381_G1_MSM);
    bls_file_test!(bls_msm_g2, "msm_G2_bls", PRECOMPILE_BLS12_381_G2_MSM);

    // Scalar mul via MSM precompiles.
    bls_file_test!(bls_mul_g1, "mul_G1_bls", PRECOMPILE_BLS12_381_G1_MSM);
    bls_file_test!(bls_mul_g2, "mul_G2_bls", PRECOMPILE_BLS12_381_G2_MSM);

    bls_file_test!(
        bls_pairing_check,
        "pairing_check_bls",
        PRECOMPILE_BLS12_381_PAIRING
    );
}

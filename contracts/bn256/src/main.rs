#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    entrypoint,
    Bytes,
    ContextReader,
    ExitCode,
    SharedAPI,
    PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL,
    PRECOMPILE_BN256_PAIR,
};
use revm_precompile::bn128::{
    add::ISTANBUL_ADD_GAS_COST,
    mul::ISTANBUL_MUL_GAS_COST,
    pair::{ISTANBUL_PAIR_BASE, ISTANBUL_PAIR_PER_POINT},
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
    let result = match bytecode_address {
        PRECOMPILE_BN256_ADD => {
            revm_precompile::bn128::run_add(&input, ISTANBUL_ADD_GAS_COST, gas_limit)
        }
        PRECOMPILE_BN256_MUL => {
            revm_precompile::bn128::run_mul(&input, ISTANBUL_MUL_GAS_COST, gas_limit)
        }
        PRECOMPILE_BN256_PAIR => revm_precompile::bn128::run_pair(
            &input,
            ISTANBUL_PAIR_PER_POINT,
            ISTANBUL_PAIR_BASE,
            gas_limit,
        ),
        _ => unreachable!("bn128: unsupported contract address"),
    };
    let result = result.unwrap_or_else(|err| sdk.exit(ExitCode::from(err)));
    sdk.sync_evm_gas(result.gas_used, 0);
    // write output
    sdk.write(result.bytes.as_ref());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_sdk_testing::HostTestingContext;

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
    fn test_chfast1() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!("18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f3726607c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7"),
            &hex!("2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915"),
            150,
        );
    }

    #[test]
    fn test_chfast2() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!("2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c91518b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f37266"),
            &hex!("2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb721611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb204"),
            150,
        );
    }

    #[test]
    fn test_cdetrio1() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            150,
        );
    }

    #[test]
    fn test_cdetrio2() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            150,
        );
    }

    #[test]
    fn test_cdetrio3() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            150,
        );
    }

    #[test]
    fn test_cdetrio4() {
        exec_evm_precompile(
            PRECOMPILE_BN256_ADD,
            &hex!(""),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            150,
        );
    }
}

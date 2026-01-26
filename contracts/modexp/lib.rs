#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{system_entrypoint2, Bytes, ContextReader, ExitCode, SharedAPI};

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // read full input data
    let gas_limit = sdk.context().contract_gas_limit();
    let input = sdk.input();
    // call modexp function
    let result = revm_precompile::modexp::berlin_run(&input, gas_limit)
        .map_err(|_| ExitCode::PrecompileError)?;
    sdk.sync_evm_gas(result.gas_used)?;
    // write output
    sdk.write(result.bytes);
    Ok(())
}

system_entrypoint2!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::TestingContextImpl;

    fn exec_evm_precompile(inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000_000;
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
    fn eip_example1() {
        exec_evm_precompile(
            &hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000002003fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2efffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
            1360,
        );
    }

    #[test]
    fn eip_example2() {
        exec_evm_precompile(
            &hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000020fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2efffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000"),
            1360,
        );
    }

    #[test]
    fn nagydani_1_square() {
        exec_evm_precompile(
            &hex!("000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000040e09ad9675465c53a109fac66a445c91b292d2bb2c5268addb30cd82f80fcb0033ff97c80a5fc6f39193ae969c6ede6710a6b7ac27078a06d90ef1c72e5c85fb502fc9e1f6beb81516545975218075ec2af118cd8798df6e08a147c60fd6095ac2bb02c2908cf4dd7c81f11c289e4bce98f3553768f392a80ce22bf5c4f4a248c6b"),
            &hex!("60008f1614cc01dcfb6bfb09c625cf90b47d4468db81b5f8b7a39d42f332eab9b2da8f2d95311648a8f243f4bb13cfb3d8f7f2a3c014122ebb3ed41b02783adc"),
            200,
        );
    }

    #[test]
    fn nagydani_1_qube() {
        exec_evm_precompile(
            &hex!("000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000040e09ad9675465c53a109fac66a445c91b292d2bb2c5268addb30cd82f80fcb0033ff97c80a5fc6f39193ae969c6ede6710a6b7ac27078a06d90ef1c72e5c85fb503fc9e1f6beb81516545975218075ec2af118cd8798df6e08a147c60fd6095ac2bb02c2908cf4dd7c81f11c289e4bce98f3553768f392a80ce22bf5c4f4a248c6b"),
            &hex!("4834a46ba565db27903b1c720c9d593e84e4cbd6ad2e64b31885d944f68cd801f92225a8961c952ddf2797fa4701b330c85c4b363798100b921a1a22a46a7fec"),
            200,
        );
    }
}

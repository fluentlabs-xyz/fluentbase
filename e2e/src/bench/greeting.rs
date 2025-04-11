extern crate test;

use crate::{examples::EXAMPLE_GREETING, utils::EvmTestingContext};
use fluentbase_sdk::{Address, Bytes};
use hex_literal::hex;
use test::Bencher;

#[bench]
fn bench_evm_greeting(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../../assets/HelloWorld.bin"))
            .unwrap()
            .into(),
    );

    let hello_world = |ctx: &mut EvmTestingContext| {
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("45773e4e").into(),
            None,
            None,
        );
        assert!(result.is_success());
        assert_eq!(
            &result.output().cloned().unwrap_or_default().as_ref()[64..76],
            "Hello, World".as_bytes()
        );
    };

    b.iter(|| {
        hello_world(&mut ctx);
    });
}

#[bench]
fn bench_wasm_greeting(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        EXAMPLE_GREETING.into(),
    );

    let hello_world = |ctx: &mut EvmTestingContext| {
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            Bytes::default(),
            None,
            None,
        );
        assert!(result.is_success());
        assert_eq!(
            result.output().cloned().unwrap_or_default().as_ref(),
            "Hello, World".as_bytes()
        );
    };

    b.iter(|| {
        hello_world(&mut ctx);
    });
}

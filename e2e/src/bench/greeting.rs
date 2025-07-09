extern crate test;

use crate::{EvmTestingContextWithGenesis, EXAMPLE_GREETING};
use fluentbase_sdk::{Address, Bytes};
use fluentbase_sdk_testing::EvmTestingContext;
use hex_literal::hex;
use test::Bencher;

#[bench]
fn bench_evm_greeting(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../../assets/HelloWorld.bin"))
            .unwrap()
            .into(),
    );

    let hello_world = |ctx: &mut EvmTestingContext| {
        ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("45773e4e").into(),
            None,
            None,
        );
    };

    b.iter(|| {
        hello_world(&mut ctx);
    });
}

#[bench]
fn bench_wasm_greeting(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, EXAMPLE_GREETING.into());

    let hello_world = |ctx: &mut EvmTestingContext| {
        ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            Bytes::default(),
            None,
            None,
        );
    };

    b.iter(|| {
        hello_world(&mut ctx);
    });
}

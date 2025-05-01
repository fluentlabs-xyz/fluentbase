extern crate test;

use crate::{utils::EvmTestingContext, EXAMPLE_ERC20};
use fluentbase_sdk::Address;
use hex_literal::hex;
use test::Bencher;

#[bench]
fn bench_evm_erc20(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../../assets/ERC20.bin"))
            .unwrap()
            .into(),
    );

    let transfer_coin = |ctx: &mut EvmTestingContext| {
        ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into(),
            None,
            None,
        );
    };

    b.iter(|| {
        transfer_coin(&mut ctx);
    });
}

#[bench]
fn bench_wasm_erc20(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, EXAMPLE_ERC20.into());

    let transfer_coin = |ctx: &mut EvmTestingContext| {
        ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into(),
            None,
            None,
        );
    };

    b.iter(|| {
        transfer_coin(&mut ctx);
    });
}

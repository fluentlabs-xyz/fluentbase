extern crate test;

use crate::EXAMPLE_ERC20;
use fluentbase_erc20::{
    common::fixed_bytes_from_u256,
    consts::SIG_TRANSFER,
    storage::{Feature, InitialSettings, DECIMALS_DEFAULT},
};
use fluentbase_sdk::{address, Address, Bytes, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_types::{ContractContextV1, PRECOMPILE_ERC20_RUNTIME};
use hex_literal::hex;
use test::Bencher;

#[bench]
fn bench_original_evm_erc20(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    ctx.disabled_rwasm = true;
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
fn bench_emulated_evm_erc20(b: &mut Bencher) {
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
fn bench_rwasm_contract_erc20(b: &mut Bencher) {
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

#[bench]
fn bench_precompiled_erc20(b: &mut Bencher) {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDR: Address = address!("1111111111111111111111111111111111111111");
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_ERC20_RUNTIME,
        ..Default::default()
    });
    let mut initial_settings = InitialSettings::new();
    let total_supply = U256::from(0xffff_ffffu64);
    initial_settings.add_feature(Feature::InitialSupply {
        amount: fixed_bytes_from_u256(&total_supply),
        owner: DEPLOYER_ADDR.into(),
        decimals: DECIMALS_DEFAULT,
    });
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDR,
        initial_settings
            .try_encode_for_deploy()
            .expect("failed to encode settings for deployment")
            .into(),
    );

    #[inline(always)]
    fn call_with_sig(
        ctx: &mut EvmTestingContext,
        input: Bytes,
        caller: &Address,
        callee: &Address,
    ) {
        ctx.call_evm_tx(*caller, *callee, input, None, None);
    }

    let to = address!("0000000000000000000000000000000000000000");
    let amount = U256::from(22);
    let mut input = Vec::<u8>::new();
    input.extend(SIG_TRANSFER.to_le_bytes());
    input.extend(to.as_slice());
    input.extend(&fixed_bytes_from_u256(&amount));
    let input: Bytes = input.clone().into();
    println!("SIG_TRANSFER input hex: {}", hex::encode(&input));

    let transfer_coin = |ctx: &mut EvmTestingContext| {
        let input = hex!("bb9c05a900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000016").into();
        call_with_sig(ctx, input, &DEPLOYER_ADDR, &contract_address);
    };

    // warmup
    transfer_coin(&mut ctx);
    b.iter(|| {
        transfer_coin(&mut ctx);
    });
}

// evm erc20: 493625.125 ns/iter (+/- 14337.172499999986)
// wasm erc20: 136839.875 ns/iter (+/- 6614.3399999999965)
// native erc20 (be u256): 133378.0925 ns/iter (+/- 4874.236499999999)
// native erc20 (le u256): 129444.015625 ns/iter (+/- 4271.5389062500035)

// native erc20 (le u256): 80671.52 ns/iter (+/- 3405.3866249999846) 29% faster wasm erc20
// native erc20 (be u256): 43017.572916666664 ns/iter (+/- 2429.1224305555515) 68% faster wasm erc20

use criterion::{criterion_group, criterion_main, Criterion};
use fluentbase_e2e::EXAMPLE_ERC20;
use fluentbase_erc20::{
    common::fixed_bytes_from_u256,
    storage::{Feature, InitialSettings, DECIMALS_DEFAULT},
};
use fluentbase_sdk::{address, Address, Bytes, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_types::{ContractContextV1, PRECOMPILE_ERC20_RUNTIME};
use hex_literal::hex;

fn erc20_transfer_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("ERC20 Transfer Comparison");

    // --- Benchmark 1: Original EVM ERC20 (rWasm disabled) ---
    {
        let mut ctx = EvmTestingContext::default();
        ctx.disabled_rwasm = true;
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/ERC20.bin"))
                .unwrap()
                .into(),
        );
        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("1_Original_EVM_ERC20", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 2: Emulated EVM ERC20 (rWasm enabled) ---
    {
        let mut ctx = EvmTestingContext::default();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/ERC20.bin"))
                .unwrap()
                .into(),
        );
        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("2_Emulated_EVM_ERC20", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 3: rWasm Contract ERC20 ---
    {
        let mut ctx = EvmTestingContext::default();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, EXAMPLE_ERC20.into());
        let transfer_payload: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();

        group.bench_function("3_rWasm_Contract_ERC20", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 4: Precompiled ERC20 ---
    {
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

        let transfer_payload: Bytes = hex!("bb9c05a900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000016").into();

        group.bench_function("4_Precompiled_ERC20", |b| {
            // Note: Manual warmup calls are not needed. Criterion handles warmups automatically.
            b.iter(|| {
                ctx.call_evm_tx(
                    DEPLOYER_ADDR,
                    contract_address,
                    transfer_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    group.finish();
}

criterion_group!(benches, erc20_transfer_benches);
criterion_main!(benches);

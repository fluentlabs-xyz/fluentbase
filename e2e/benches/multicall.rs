use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use fluentbase_e2e::{EvmTestingContextWithGenesis, EXAMPLE_ROUTER_SOLIDITY};
use fluentbase_sdk::{Address, Bytes};
use fluentbase_sdk_testing::EvmTestingContext;
use hex_literal::hex;

fn multicall_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("Multicall Router Comparison");

    // This payload encodes a multicall that invokes two functions on a target contract:
    // 1. greeting("Hello, World!")
    // 2. customGreeting("Custom Hello, World!")
    let multicall_payload: Bytes = black_box(hex!("ac9650d800000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000064f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c64212100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006436b83a9a00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000015437573746f6d2048656c6c6f2c20576f726c642121000000000000000000000000").into());
    const GAS_LIMIT: Option<u64> = Some(300_000_000);

    // --- Benchmark 1: EVM Multicall ---
    {
        let mut ctx = EvmTestingContext::default()
            .with_full_genesis();
        const DEPLOYER_ADDRESS: Address = Address::ZERO;
        let router_address = ctx.deploy_evm_tx(
            DEPLOYER_ADDRESS,
            hex::decode(include_bytes!("../assets/Router.bin"))
                .unwrap()
                .into(),
        );

        group.bench_function("EVM_Multicall", |b| {
            b.iter(|| {
                // The `assert!` is kept to ensure the benchmark measures a successful execution.
                let result = ctx.call_evm_tx(
                    DEPLOYER_ADDRESS,
                    router_address,
                    multicall_payload.clone(),
                    GAS_LIMIT,
                    None,
                );
                assert!(result.is_success());
            });
        });
    }

    // --- Benchmark 2: Wasm Multicall ---
    {
        let mut ctx = EvmTestingContext::default()
            .with_full_genesis();
        const DEPLOYER_ADDRESS: Address = Address::ZERO;
        let router_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_ROUTER_SOLIDITY.into());

        group.bench_function("WASM_Multicall", |b| {
            b.iter(|| {
                let result = ctx.call_evm_tx(
                    DEPLOYER_ADDRESS,
                    router_address,
                    multicall_payload.clone(),
                    GAS_LIMIT,
                    None,
                );
                assert!(result.is_success());
            });
        });
    }

    group.finish();
}

criterion_group!(benches, multicall_benches);
criterion_main!(benches);

use criterion::{criterion_group, criterion_main, Criterion};
use fluentbase_contracts::FLUENTBASE_EXAMPLES_GREETING;
use fluentbase_e2e::EvmTestingContextWithGenesis;
use fluentbase_sdk::{Address, Bytes};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;

/// A benchmark suite for comparing EVM and Wasm "greeting" contracts.
fn greeting_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("Greeting Contract Comparison");

    // --- Benchmark 1: Original "Hello World" Contract ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.disabled_rwasm = true;
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/HelloWorld.bin"))
                .unwrap()
                .into(),
        );
        let call_payload: Bytes = hex!("45773e4e").into();

        group.bench_function("Original_Greeting", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    call_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 2: EVM "Hello World" Contract ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            hex::decode(include_bytes!("../assets/HelloWorld.bin"))
                .unwrap()
                .into(),
        );
        let call_payload: Bytes = hex!("45773e4e").into();

        group.bench_function("EVM_Greeting", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    call_payload.clone(),
                    None,
                    None,
                );
            });
        });
    }

    // --- Benchmark 3: Wasm "Greeting" Contract ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(
            OWNER_ADDRESS,
            FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into(),
        );

        group.bench_function("WASM_Greeting", |b| {
            b.iter(|| {
                ctx.call_evm_tx(
                    OWNER_ADDRESS,
                    contract_address,
                    // This call uses an empty payload.
                    Bytes::default(),
                    None,
                    None,
                );
            });
        });
    }

    group.finish();
}

criterion_group!(benches, greeting_benches);
criterion_main!(benches);

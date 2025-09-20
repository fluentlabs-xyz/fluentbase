use criterion::{criterion_group, criterion_main, Criterion};
use fluentbase_e2e::{EvmTestingContextWithGenesis, EXAMPLE_GREETING};
use fluentbase_sdk::{Address, Bytes};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;

/// A benchmark suite for comparing EVM and Wasm "greeting" contracts.
fn greeting_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("Greeting Contract Comparison");

    // --- Benchmark 1: EVM "Hello World" Contract ---
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

    // --- Benchmark 2: Wasm "Greeting" Contract ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;
        let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, EXAMPLE_GREETING.into());

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

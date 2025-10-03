use criterion::{criterion_main, Criterion};
use fluentbase_e2e::{EvmTestingContextWithGenesis, EXAMPLE_FIB};
use fluentbase_sdk::Address;
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;
use std::time::Duration;

const FIB_VALUE: i32 = 43;

const PAYLOAD: [u8; 36] = {
    let mut payload: [u8; 36] = [0; 36];
    let selector = hex!("5d6132a8");
    let value_bytes = FIB_VALUE.to_be_bytes();

    payload[0] = selector[0];
    payload[1] = selector[1];
    payload[2] = selector[2];
    payload[3] = selector[3];

    payload[32] = value_bytes[0];
    payload[33] = value_bytes[1];
    payload[34] = value_bytes[2];
    payload[35] = value_bytes[3];

    payload
};
const EXPECTED_BYTES: [u8; 32] =
    hex!("0000000000000000000000000000000000000000000000000000000019d699a5");
const EXPECTED: i32 = 433494437;

fn fibonacci_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("Fib Comparison");

    // --- Benchmark 0: Native Rust Fib ---
    {
        pub fn fib(n: i32) -> i32 {
            let (mut a, mut b) = (0, 1);
            for _ in 0..n {
                let t = a;
                a = b;
                b = t + b;
            }
            a
        }

        group.bench_function("0_Native_Rust_Fib", |b| {
            b.iter(|| {
                let answer = core::hint::black_box(fib(core::hint::black_box(FIB_VALUE)));
                assert_eq!(answer, EXPECTED);
            });
        });
    }

    // --- Benchmark 2: Original EVM Solidity Fib (rWasm disabled) ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.disabled_rwasm = true;
        const OWNER_ADDRESS: Address = Address::ZERO;

        let code = hex::decode(include_bytes!("../assets/Fib.bin")).unwrap();

        let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, code.into());

        group.bench_function("2_Original_EVM_Solidity_Fib", |b| {
            b.iter(|| {
                let result =
                    ctx.call_evm_tx(OWNER_ADDRESS, contract_address, PAYLOAD.into(), None, None);
                assert!(result.is_success());
                let answer = result.output().unwrap();

                assert_eq!(&answer[..], &EXPECTED_BYTES[..]);
            });
        });
    }

    // --- Benchmark 3: RWASM ---
    {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const OWNER_ADDRESS: Address = Address::ZERO;

        let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, EXAMPLE_FIB.into());

        group.bench_function("3_WASM_Fib", |b| {
            b.iter(|| {
                let result =
                    ctx.call_evm_tx(OWNER_ADDRESS, contract_address, PAYLOAD.into(), None, None);
                assert!(result.is_success());
                let answer = result.output().unwrap();
                assert_eq!(&answer[..], &EXPECTED.to_be_bytes()[..]);
            });
        });
    }

    group.finish();
}

pub fn benches() {
    let mut criterion: Criterion<_> = Criterion::default()
        .configure_from_args()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(1))
        .sample_size(1000);
    fibonacci_benches(&mut criterion);
}

criterion_main!(benches);

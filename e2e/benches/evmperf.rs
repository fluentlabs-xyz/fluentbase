//! Throughput comparison of native revm against the rWasm-hosted EVM runtime on workloads that
//! isolate the cost centers: pure interpretation, hashing syscalls, storage interruptions and a
//! realistic ERC20 transfer.
//!
//! Run everything with `cargo bench -p fluentbase-e2e --bench evmperf --profile release`, or one
//! case with `EVMPERF_CASE=<name> EVMPERF_MODE=<native|rwasm|reset> EVMPERF_ITERS=<n>` (the
//! `reset` mode drops the cached system runtime before every call, as the node does per block).
use fluentbase_e2e::EvmTestingContextWithGenesis;
use fluentbase_runtime::runtime::SystemRuntime;
use fluentbase_sdk::{Address, Bytes};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use std::time::Instant;

const OWNER: Address = Address::ZERO;

fn initcode_for(code: &[u8]) -> Bytes {
    assert!(code.len() < 256);
    let mut init = vec![
        0x60,
        code.len() as u8,
        0x60,
        0x0c,
        0x60,
        0x00,
        0x39,
        0x60,
        code.len() as u8,
        0x60,
        0x00,
        0xf3,
    ];
    init.extend_from_slice(code);
    init.into()
}

/// Runtime code that repeats `body` `n` times:
/// `PUSH2 n; JUMPDEST; <body>; PUSH1 1; SWAP1; SUB; DUP1; PUSH1 3; JUMPI; STOP`.
fn loop_code(n: u16, body: &[u8]) -> Vec<u8> {
    let mut code = vec![0x61, (n >> 8) as u8, n as u8, 0x5b];
    code.extend_from_slice(body);
    code.extend_from_slice(&[0x60, 0x01, 0x90, 0x03, 0x80, 0x60, 0x03, 0x57, 0x00]);
    code
}

fn deploy(ctx: &mut EvmTestingContext, init: Bytes) -> Address {
    let nonce = ctx.nonce(OWNER);
    let result = TxBuilder::create(ctx, OWNER, init)
        .gas_limit(10_000_000)
        .exec();
    assert!(result.is_success(), "deploy failed: {result:?}");
    OWNER.create(nonce)
}

struct Case {
    name: &'static str,
    init: Bytes,
    input: Bytes,
    iters: usize,
}

fn run_case(native: bool, reset_per_call: bool, case: &Case) -> (f64, u64) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.disabled_rwasm = native;
    let addr = deploy(&mut ctx, case.init.clone());
    // Warm up caches (compiled modules, instantiated runtimes, journal state).
    for _ in 0..3 {
        let r = ctx.call_evm_tx(OWNER, addr, case.input.clone(), Some(3_000_000), None);
        assert!(r.is_success(), "{}: {:?}", case.name, r);
    }
    let mut gas = 0u64;
    let start = Instant::now();
    for _ in 0..case.iters {
        if reset_per_call {
            SystemRuntime::reset_cached_runtimes();
        }
        let r = ctx.call_evm_tx(OWNER, addr, case.input.clone(), Some(3_000_000), None);
        gas += r.tx_gas_used();
    }
    let secs = start.elapsed().as_secs_f64();
    (secs / case.iters as f64 * 1e6, gas / case.iters as u64)
}

fn main() {
    let transfer: Bytes = hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into();
    let cases = vec![
        Case {
            name: "erc20_transfer",
            init: hex::decode(include_bytes!("../assets/ERC20.bin"))
                .unwrap()
                .into(),
            input: transfer,
            iters: 2000,
        },
        Case {
            name: "arith_loop_20k",
            init: initcode_for(&loop_code(20_000, &[])),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "keccak_loop_10k",
            // PUSH1 32 PUSH1 0 KECCAK256 POP
            init: initcode_for(&loop_code(10_000, &[0x60, 0x20, 0x60, 0x00, 0x20, 0x50])),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "sload_loop_2k",
            // PUSH1 0 SLOAD POP
            init: initcode_for(&loop_code(2_000, &[0x60, 0x00, 0x54, 0x50])),
            input: Bytes::new(),
            iters: 100,
        },
        Case {
            name: "sstore_loop_2k",
            // DUP1 PUSH1 0 SSTORE
            init: initcode_for(&loop_code(2_000, &[0x80, 0x60, 0x00, 0x55])),
            input: Bytes::new(),
            iters: 100,
        },
    ];
    if let Ok(only) = std::env::var("EVMPERF_CASE") {
        let mode = std::env::var("EVMPERF_MODE").unwrap_or_else(|_| "rwasm".into());
        let iters: usize = std::env::var("EVMPERF_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);
        let case = cases.iter().find(|c| c.name == only).expect("unknown case");
        let case = Case {
            name: case.name,
            init: case.init.clone(),
            input: case.input.clone(),
            iters,
        };
        let (us, gas) = run_case(mode == "native", mode == "reset", &case);
        println!(
            "{} {} iters={} gas={} avg_us={:.1} Mgas/s={:.1}",
            case.name,
            mode,
            iters,
            gas,
            us,
            gas as f64 / us
        );
        return;
    }
    println!(
        "{:<18} {:>10} {:>12} {:>12} {:>12} {:>10} {:>10}",
        "case", "gas/tx", "native_us", "rwasm_us", "rwasm_reset", "slowdown", "rwasm_Mgas/s"
    );
    for case in &cases {
        let (nat_us, gas) = run_case(true, false, case);
        let (rw_us, gas2) = run_case(false, false, case);
        let (rw_reset_us, _) = run_case(false, true, case);
        assert_eq!(gas, gas2, "gas mismatch for {}", case.name);
        println!(
            "{:<18} {:>10} {:>12.1} {:>12.1} {:>12.1} {:>9.1}x {:>10.1}",
            case.name,
            gas,
            nat_us,
            rw_us,
            rw_reset_us,
            rw_us / nat_us,
            gas as f64 / rw_us
        );
    }
}

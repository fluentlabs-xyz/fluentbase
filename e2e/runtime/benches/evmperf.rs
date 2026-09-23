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

/// `PUSH32 <word>`.
fn push32(word: [u8; 32]) -> Vec<u8> {
    let mut code = vec![0x7f];
    code.extend_from_slice(&word);
    code
}

/// `<op> a b` with `a` on top of the stack, result popped: `PUSH32 b; PUSH32 a; <op>; POP`.
fn binop_body(op: u8, a: [u8; 32], b: [u8; 32]) -> Vec<u8> {
    [push32(b), push32(a), vec![op, 0x50]].concat()
}

/// `<op> a b n` (`ADDMOD`/`MULMOD`), result popped.
fn ternop_body(op: u8, a: [u8; 32], b: [u8; 32], n: [u8; 32]) -> Vec<u8> {
    [push32(n), push32(b), push32(a), vec![op, 0x50]].concat()
}

// Full-width 256-bit operands, every limb populated, so the arithmetic takes its general path.
const WORD_A: [u8; 32] = hex!("d3b1a95c7e2f4680b9c1d8e6f5a4037c2e9d1b6a8f4c3e7d5b2a9c8e1f6d4b3a");
const WORD_B: [u8; 32] = hex!("6f2e8a1d4c9b7e3a5d8f1c6b2e9a4d7f3b5c8e1a6d2f9b4c7e3a1d5f8b2c6e9a");
// ~2^255 modulus for `ADDMOD`/`MULMOD`
const WORD_N: [u8; 32] = hex!("fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");
// ~2^130 divisor: a multi-limb divisor on a 4-limb dividend takes the long-division path
const WORD_D: [u8; 32] = hex!("0000000000000000000000000000000497a3f1c5d2e8b6f4a1c3e5d7b9f2a4c6");

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
        // 256-bit arithmetic on full-width operands: the EVM runtime's `ruint` limb products and
        // carry chains, which the wide-arithmetic instructions lower directly.
        Case {
            name: "add256_loop_20k",
            init: initcode_for(&loop_code(20_000, &binop_body(0x01, WORD_A, WORD_B))),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "mul256_loop_20k",
            init: initcode_for(&loop_code(20_000, &binop_body(0x02, WORD_A, WORD_B))),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "div256_loop_10k",
            init: initcode_for(&loop_code(10_000, &binop_body(0x04, WORD_A, WORD_D))),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "addmod256_loop_10k",
            init: initcode_for(&loop_code(10_000, &ternop_body(0x08, WORD_A, WORD_B, WORD_N))),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            name: "mulmod256_loop_10k",
            init: initcode_for(&loop_code(10_000, &ternop_body(0x09, WORD_A, WORD_B, WORD_N))),
            input: Bytes::new(),
            iters: 200,
        },
        Case {
            // `PUSH8 e; PUSH32 a; EXP; POP`: a 64-bit exponent is 64 squarings and multiplies
            name: "exp256_loop_2k",
            init: initcode_for(&loop_code(
                2_000,
                &[
                    vec![0x67, 0x9b, 0x3d, 0xe7, 0x51, 0xa2, 0xc4, 0x8f, 0x6d],
                    push32(WORD_A),
                    vec![0x0a, 0x50],
                ]
                .concat(),
            )),
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

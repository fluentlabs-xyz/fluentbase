//! Round-trip cost of the fine-grained BLS12-381 syscalls on this build's system-runtime
//! backend: a base-field multiplication (`_tower_fp1_bls12381_mul`), an Fp2 multiplication
//! (`_tower_fp2_bls12381_mul`) and an affine G1 doubling (`_bls12381_double`, one field
//! inversion on the host), against a bare host call. A pairing is thousands of these, so the
//! numbers bound what a syscall-based BLS guest can reach. Ignored by default; run with
//! `--ignored --nocapture`.

use fluentbase_runtime::{runtime::SystemRuntime, RuntimeContext};
use fluentbase_sdk::{import_linker_v1_preview, Address, B256};
use hex_literal::hex;
use rwasm::{RwasmModule, RwasmModuleInner};
use std::time::Instant;

const ITERATIONS: u32 = 200_000;

/// EIP-2537 G1 generator, big-endian coordinates.
const G1_GEN_X: [u8; 48] = hex!("17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb");
const G1_GEN_Y: [u8; 48] = hex!("08b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1");

#[derive(Clone, Copy)]
enum Op {
    Bare,
    Fp1Mul,
    Fp2Mul,
    G1Double,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Bare => "_input_size (bare host call)",
            Op::Fp1Mul => "_tower_fp1_bls12381_mul",
            Op::Fp2Mul => "_tower_fp2_bls12381_mul",
            Op::G1Double => "_bls12381_double (affine, host inversion)",
        }
    }

    fn import(self) -> &'static str {
        match self {
            Op::Bare => r#"(import "fluentbase_v1preview" "_input_size" (func $op (result i32)))"#,
            Op::Fp1Mul => {
                r#"(import "fluentbase_v1preview" "_tower_fp1_bls12381_mul" (func $op (param i32 i32)))"#
            }
            Op::Fp2Mul => {
                r#"(import "fluentbase_v1preview" "_tower_fp2_bls12381_mul" (func $op (param i32 i32 i32 i32)))"#
            }
            Op::G1Double => {
                r#"(import "fluentbase_v1preview" "_bls12381_double" (func $op (param i32)))"#
            }
        }
    }

    fn call(self) -> &'static str {
        match self {
            Op::Bare => "(drop (call $op))",
            Op::Fp1Mul => "(call $op (i32.const 0) (i32.const 128))",
            Op::Fp2Mul => "(call $op (i32.const 0) (i32.const 48) (i32.const 128) (i32.const 176))",
            Op::G1Double => "(call $op (i32.const 0))",
        }
    }

    /// Operands: field elements are 0x11 repeated (below the modulus) and the first operand is
    /// overwritten with the product each round, so nothing degenerates; the point is the G1
    /// generator in the syscall's little-endian `x || y` layout, doubled in place.
    fn data(self) -> String {
        let fill = "\\11".repeat(96);
        match self {
            Op::G1Double => {
                let mut point: Vec<u8> = Vec::with_capacity(96);
                point.extend(G1_GEN_X.iter().rev().copied());
                point.extend(G1_GEN_Y.iter().rev().copied());
                let bytes: String = point.iter().map(|b| format!("\\{b:02x}")).collect();
                format!(r#"(data (i32.const 0) "{bytes}")"#)
            }
            _ => format!(r#"(data (i32.const 0) "{fill}") (data (i32.const 128) "{fill}")"#),
        }
    }
}

fn module(iterations: u32, op: Op) -> RwasmModule {
    let wat = format!(
        r#"(module
            {import}
            (memory (export "memory") 1)
            {data}
            (func (export "main") (param i32 i32) (result i32)
                (local $i i32)
                (local.set $i (i32.const {iterations}))
                (block $done
                    (br_if $done (i32.eqz (local.get $i)))
                    (loop $l
                        {call}
                        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
                        (br_if $l (local.get $i))))
                (i32.const 0)))"#,
        import = op.import(),
        data = op.data(),
        call = op.call(),
    );
    RwasmModuleInner {
        hint_section: wat::parse_str(&wat).expect("wat compiles"),
        ..Default::default()
    }
    .into()
}

fn run(iterations: u32, op: Op) -> f64 {
    SystemRuntime::reset_cached_runtimes();
    let mut runtime = SystemRuntime::new(
        module(iterations, op),
        import_linker_v1_preview(),
        B256::with_last_byte((iterations as u8).wrapping_add(op as u8 * 50 + 1)),
        Address::ZERO,
        RuntimeContext::default().with_fuel_limit(u64::MAX / 4),
        false,
    )
    .expect("system runtime loads");
    let started = Instant::now();
    runtime.execute().expect("execute");
    started.elapsed().as_secs_f64()
}

#[test]
#[ignore = "syscall cost micro-benchmark; run manually with --ignored --nocapture"]
fn bls_fp_mul_syscall_round_trip() {
    println!("backend: {}", super::bls12381::BACKEND);
    for op in [Op::Fp1Mul, Op::Fp2Mul, Op::G1Double, Op::Bare] {
        let iterations = if matches!(op, Op::G1Double) {
            ITERATIONS / 10
        } else {
            ITERATIONS
        };
        let empty = run(0, op);
        let full = run(iterations, op);
        println!(
            "{}: {iterations} calls in {:.3} s (empty run {:.6} s) -> {:.0} ns per syscall",
            op.name(),
            full,
            empty,
            (full - empty) * 1e9 / iterations as f64
        );
    }
}

//! Round-trip cost of one BLS12-381 base-field multiplication syscall
//! (`_tower_fp1_bls12381_mul`) on this build's system-runtime backend. A pairing is tens of
//! thousands of these, so this number decides whether a syscall-accelerated BLS guest can beat
//! arkworks running inside the guest. Ignored by default; run with `--ignored --nocapture`.

use fluentbase_runtime::{runtime::SystemRuntime, RuntimeContext};
use fluentbase_sdk::{import_linker_v1_preview, Address, B256};
use rwasm::{RwasmModule, RwasmModuleInner};
use std::time::Instant;

const ITERATIONS: u32 = 200_000;

fn module(iterations: u32, bare: bool) -> RwasmModule {
    // x and y are full-width field elements (0x11 repeated, below the modulus); x is overwritten
    // with x*y each round so the operands never degenerate.
    let wat = format!(
        r#"(module
            (import "fluentbase_v1preview" "_tower_fp1_bls12381_mul" (func $mul (param i32 i32)))
            (import "fluentbase_v1preview" "_input_size" (func $input_size (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{x}")
            (data (i32.const 64) "{x}")
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
        x = "\\11".repeat(48),
        call = if bare {
            "(drop (call $input_size))"
        } else {
            "(call $mul (i32.const 0) (i32.const 64))"
        },
    );
    RwasmModuleInner {
        hint_section: wat::parse_str(&wat).expect("wat compiles"),
        ..Default::default()
    }
    .into()
}

fn run(iterations: u32, bare: bool) -> f64 {
    SystemRuntime::reset_cached_runtimes();
    let mut runtime = SystemRuntime::new(
        module(iterations, bare),
        import_linker_v1_preview(),
        B256::with_last_byte((iterations as u8).wrapping_add(if bare { 100 } else { 1 })),
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
    for (name, bare) in [
        ("_tower_fp1_bls12381_mul", false),
        ("_input_size (bare host call)", true),
    ] {
        let empty = run(0, bare);
        let full = run(ITERATIONS, bare);
        println!(
            "{name}: {ITERATIONS} calls in {:.3} s (empty run {:.6} s) -> {:.0} ns per syscall",
            full,
            empty,
            (full - empty) * 1e9 / ITERATIONS as f64
        );
    }
}

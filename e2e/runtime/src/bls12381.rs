//! Throughput of the BLS12-381 (EIP-2537) precompile, the calls that used to take seconds.
//!
//! All seven BLS addresses run one guest rWasm module (`contracts/bls12381`) built from
//! `revm-precompile`. The guest keeps the EIP gas schedule, input parsing and error semantics and
//! installs the syscall-backed `Crypto` provider (`fluentbase-precompile-crypto`), so every
//! pairing check, MSM and map crosses to the host in one `_crypto_*` syscall and runs on revm's
//! `DefaultCrypto` (blst). Only parsing and gas accounting execute in the system runtime, through
//! Wasmtime with the `wasmtime` feature or the rWasm interpreter without it, which is why both
//! flavours now perform alike. Reloading the module after a runtime-cache reset is not a factor
//! either: an rWasm compile costs milliseconds and the Wasmtime artifact is cached process-wide;
//! `system_runtime_reload_cost` measures it.
//!
//! Each test first makes a minimal warm-up call (module load and, with `wasmtime`, the cranelift
//! compile happen there and are reported separately), then times one call whose precompile gas
//! equals the legacy EIP-7825 transaction cap and asserts a lenient throughput floor. Run with
//! `--nocapture` to see the numbers.
//!
//! Measured on an M-series Mac, release build. With arkworks inside the guest (2026-09-17) the
//! 512-pair call took 0.7 s under Wasmtime (24 Mgas/s) and 260 s in the rWasm interpreter, and
//! the block-sized 3066-pair call took 4.2 s. With the syscall-backed provider (2026-09-21) the
//! 512-pair call takes about 0.1 s in both flavours (roughly 170 Mgas/s) and the 3066-pair call
//! about 0.6 s.

use crate::EvmTestingContextWithGenesis;
use fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS;
use fluentbase_runtime::{runtime::SystemRuntime, RuntimeContext};
use fluentbase_sdk::{
    calldata_quadratic_surcharge, import_linker_v1_preview, is_engine_metered_precompile,
    keccak256, Address, B256, PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_EVM_RUNTIME, PRECOMPILE_SHA256, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder, TxExecution};
use hex_literal::hex;
use rwasm::RwasmModule;
use std::time::{Duration, Instant};

/// EIP-2537 pairing schedule: `37700 + 32600 * k`.
const PAIRING_BASE_GAS: u64 = 37_700;
const PAIRING_PAIR_GAS: u64 = 32_600;
const PAIRING_PAIR_LEN: usize = 384;

/// EIP-2537 G2 MSM schedule: `k * 22500 * discount(k) / 1000`; the discount saturates at
/// `DISCOUNT_TABLE_G2_MSM[127]` for k >= 128, which is the only regime these tests use.
const G2_MSM_POINT_GAS: u64 = 22_500;
const G2_MSM_SATURATED_DISCOUNT: u64 = 524;
const MSM_MULTIPLIER: u64 = 1_000;
const G2_MSM_TERM_LEN: usize = 288;

/// EIP-7825 legacy transaction gas cap (2^24): the largest single precompile call an Ethereum
/// transaction can make. Fluent lifts the cap (`TX_GAS_LIMIT_CAP = u64::MAX`), so one
/// transaction can spend the whole block on one call; see the ignored block-sized test.
const TX_GAS_CAP: u64 = 16_777_216;

/// Genesis block gas limit (`crates/genesis/build.rs`).
const BLOCK_GAS_LIMIT: u64 = 100_000_000;

/// Throughput floor over the precompile's own gas (intrinsic calldata gas excluded).
///
/// EIP-2537 prices were calibrated on blst-class implementations that sustain tens of Mgas/s.
/// Below 5 Mgas/s a block-sized call takes over 20 s and the gas schedule no longer bounds
/// block execution time, so this is deliberately lenient: a failure means the precompile is an
/// order of magnitude off its schedule, not that the machine is slow.
const MIN_PRECOMPILE_GAS_PER_SECOND: u64 = 5_000_000;

/// Gas per non-zero calldata byte under the EIP-7623 floor; used to size the tx gas limit.
const CALLDATA_FLOOR_GAS_PER_BYTE: u64 = 40;

/// `e(G1, G2) * e(G1, -G2) = 1`: two pairs whose product is the identity, so any number of
/// repetitions is a valid pairing check that returns 1.
/// Source: contracts/bls12381/testcases/pairing_check_bls.json.
pub(crate) const PAIRING_IDENTITY_PAIRS: [u8; 2 * PAIRING_PAIR_LEN] = hex!(
    "
    0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac58
    6c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4
    fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000
    024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8
    0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049
    334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351a
    adfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b8280100000000000000000000000000000000
    0606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be
    0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac58
    6c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4
    fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000
    024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8
    0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049
    334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bc
    b679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa00000000000000000000000000000000
    13fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed
"
);

/// `(G2, 2)`: one G2 MSM term. Repeating it k times yields `2k * G2`, which is never the
/// point at infinity for the sizes used here.
/// Source: contracts/bls12381/testcases/msm_G2_bls.json.
const G2_MSM_TERM: [u8; G2_MSM_TERM_LEN] = hex!(
    "
    00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d177
    0bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65
    596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e00000000000000000000000000000000
    0ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801
    000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab
    3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000000000000000000000000000000000002
"
);

struct TimedCall {
    execution: TxExecution,
    elapsed: Duration,
}

/// Which backend the system runtime uses for the BLS guest in this build; the `wasmtime`
/// feature of this crate forwards to `fluentbase-runtime/wasmtime`.
pub(crate) const BACKEND: &str = if cfg!(feature = "wasmtime") {
    "wasmtime"
} else {
    "rwasm interpreter"
};

fn context() -> EvmTestingContext {
    println!("bls backend: {BACKEND}");
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(Address::ZERO, U256::from(u128::MAX));
    ctx
}

/// Sends `input` straight to `precompile` as a transaction from `Address::ZERO` and times the
/// whole transaction. The tx gas limit covers the precompile gas, the EIP-7623 calldata floor
/// and Fluent's quadratic calldata surcharge; the block limit is lifted so block-sized calls
/// are accepted.
fn timed_call(
    ctx: &mut EvmTestingContext,
    precompile: Address,
    input: Vec<u8>,
    precompile_gas: u64,
) -> TimedCall {
    let input_len = input.len() as u64;
    let gas_limit = precompile_gas
        + input_len * CALLDATA_FLOOR_GAS_PER_BYTE
        + calldata_quadratic_surcharge(input_len)
        + 100_000;
    let mut builder = TxBuilder::call(ctx, precompile)
        .caller(Address::ZERO)
        .input(input.into())
        .gas_limit(gas_limit);
    builder.block.gas_limit = u64::MAX;
    let started = Instant::now();
    let execution = builder.execute();
    TimedCall {
        execution,
        elapsed: started.elapsed(),
    }
}

fn pairing_input(pairs: usize) -> Vec<u8> {
    assert!(pairs % 2 == 0, "the identity template holds two pairs");
    PAIRING_IDENTITY_PAIRS.repeat(pairs / 2)
}

fn pairing_gas(pairs: usize) -> u64 {
    PAIRING_BASE_GAS + PAIRING_PAIR_GAS * pairs as u64
}

/// Largest even pair count whose precompile gas fits in `gas`.
fn pairs_for_gas(gas: u64) -> usize {
    (((gas - PAIRING_BASE_GAS) / PAIRING_PAIR_GAS) as usize) & !1
}

fn g2_msm_input(points: usize) -> Vec<u8> {
    G2_MSM_TERM.repeat(points)
}

fn g2_msm_gas(points: usize) -> u64 {
    assert!(
        points >= 128,
        "the saturated discount only applies from k = 128"
    );
    points as u64 * G2_MSM_POINT_GAS * G2_MSM_SATURATED_DISCOUNT / MSM_MULTIPLIER
}

/// Largest point count whose precompile gas fits in `gas`.
fn points_for_gas(gas: u64) -> usize {
    (gas / (G2_MSM_POINT_GAS * G2_MSM_SATURATED_DISCOUNT / MSM_MULTIPLIER)) as usize
}

fn report_and_assert_throughput(name: &str, units: usize, precompile_gas: u64, call: &TimedCall) {
    let secs = call.elapsed.as_secs_f64();
    println!(
        "{name}: {units} units, {precompile_gas} precompile gas of {} tx gas, {secs:.3} s, \
         {:.1} Mgas/s, {:.2} ms/unit",
        call.execution.gas_used(),
        precompile_gas as f64 / secs / 1e6,
        secs * 1e3 / units as f64,
    );
    let budget =
        Duration::from_secs_f64(precompile_gas as f64 / MIN_PRECOMPILE_GAS_PER_SECOND as f64);
    assert!(
        call.elapsed <= budget,
        "{name} took {secs:.3} s for {precompile_gas} precompile gas, over the {:.3} s budget at \
         {} gas/s; the BLS guest is executing far below its EIP-2537 gas schedule",
        budget.as_secs_f64(),
        MIN_PRECOMPILE_GAS_PER_SECOND,
    );
}

fn run_pairing(precompile_gas_budget: u64) {
    let mut ctx = context();

    let warmup = timed_call(
        &mut ctx,
        PRECOMPILE_BLS12_381_PAIRING,
        pairing_input(2),
        pairing_gas(2),
    );
    warmup
        .execution
        .expect_ok()
        .expect_output(B256::with_last_byte(1));
    println!(
        "bls pairing warm-up (cold runtime, 2 pairs): {:.3} s",
        warmup.elapsed.as_secs_f64()
    );

    let pairs = pairs_for_gas(precompile_gas_budget);
    let precompile_gas = pairing_gas(pairs);
    let call = timed_call(
        &mut ctx,
        PRECOMPILE_BLS12_381_PAIRING,
        pairing_input(pairs),
        precompile_gas,
    );
    call.execution
        .expect_ok()
        .expect_output(B256::with_last_byte(1));
    assert!(call.execution.gas_used() >= precompile_gas);
    report_and_assert_throughput("bls pairing", pairs, precompile_gas, &call);
}

fn run_g2_msm(precompile_gas_budget: u64) {
    let mut ctx = context();

    let warmup = timed_call(
        &mut ctx,
        PRECOMPILE_BLS12_381_G2_MSM,
        g2_msm_input(128),
        g2_msm_gas(128),
    );
    warmup.execution.expect_ok();
    println!(
        "bls g2 msm warm-up (cold runtime, 128 points): {:.3} s",
        warmup.elapsed.as_secs_f64()
    );

    let points = points_for_gas(precompile_gas_budget);
    let precompile_gas = g2_msm_gas(points);
    let call = timed_call(
        &mut ctx,
        PRECOMPILE_BLS12_381_G2_MSM,
        g2_msm_input(points),
        precompile_gas,
    );
    let output = call
        .execution
        .expect_ok()
        .output()
        .expect("g2 msm returns a point");
    assert_eq!(output.len(), 256, "g2 msm returns one padded G2 point");
    assert!(
        output.iter().any(|byte| *byte != 0),
        "2k * G2 must not be the point at infinity"
    );
    assert!(call.execution.gas_used() >= precompile_gas);
    report_and_assert_throughput("bls g2 msm", points, precompile_gas, &call);
}

/// One pairing call at the EIP-7825 transaction cap: 512 pairs, ~16.7M precompile gas.
#[test]
fn bls12_381_pairing_at_tx_gas_cap_meets_throughput_floor() {
    run_pairing(TX_GAS_CAP);
}

/// One G2 MSM call at the EIP-7825 transaction cap: ~1400 points, ~16.7M precompile gas.
#[test]
fn bls12_381_g2_msm_at_tx_gas_cap_meets_throughput_floor() {
    run_g2_msm(TX_GAS_CAP);
}

/// Worst case a single Fluent transaction can submit: one pairing call spending the whole
/// genesis block gas limit (~3000 pairs, ~1.2 MiB calldata). Even under Wasmtime this runs for
/// seconds; run it manually to see the block-level impact.
#[test]
#[ignore = "block-sized input (~3000 pairs, ~1.2 MiB calldata); run manually with --ignored"]
fn bls12_381_pairing_at_block_gas_limit() {
    run_pairing(BLOCK_GAS_LIMIT);
}

/// Times `SystemRuntime::new` for a genesis system contract: once on a cold thread-local cache,
/// once cached, and once more after `reset_cached_runtimes`, which the node calls at the start
/// of every block (`crates/node/src/evm.rs`, `apply_pre_execution_changes`).
fn time_system_runtime_load(address: Address) -> (Duration, Duration, Duration) {
    let contract = GENESIS_CONTRACTS_BY_ADDRESS
        .get(&address)
        .expect("address is a genesis contract");
    let (module, _) =
        RwasmModule::new_checked(contract.rwasm_bytecode.as_ref()).expect("genesis rwasm decodes");
    let code_hash = keccak256(contract.rwasm_bytecode.as_ref());
    let linker = import_linker_v1_preview();
    let consume_fuel = is_engine_metered_precompile(&address);
    let mut load = |module: RwasmModule| {
        let started = Instant::now();
        let runtime = SystemRuntime::new(
            module,
            linker.clone(),
            code_hash,
            address,
            RuntimeContext::default(),
            consume_fuel,
        )
        .expect("system runtime loads");
        let elapsed = started.elapsed();
        drop(runtime);
        elapsed
    };
    SystemRuntime::reset_cached_runtimes();
    let cold = load(module.clone());
    let cached = load(module.clone());
    SystemRuntime::reset_cached_runtimes();
    let after_reset = load(module);
    (cold, cached, after_reset)
}

/// What a system-runtime cache miss costs per block and per thread in this build, for the
/// modules a typical block touches, plus the same effect seen from a transaction.
#[test]
fn system_runtime_reload_cost() {
    println!("bls backend: {BACKEND}");
    for (name, address) in [
        ("evm runtime", PRECOMPILE_EVM_RUNTIME),
        ("bls pairing", PRECOMPILE_BLS12_381_PAIRING),
        ("sha256", PRECOMPILE_SHA256),
    ] {
        let (cold, cached, after_reset) = time_system_runtime_load(address);
        println!(
            "{name} SystemRuntime::new: cold {:.3} s, cached {:.6} s, after reset {:.3} s",
            cold.as_secs_f64(),
            cached.as_secs_f64(),
            after_reset.as_secs_f64()
        );
    }

    let mut ctx = context();
    let call = |ctx: &mut EvmTestingContext| {
        let call = timed_call(
            ctx,
            PRECOMPILE_BLS12_381_PAIRING,
            pairing_input(2),
            pairing_gas(2),
        );
        call.execution
            .expect_ok()
            .expect_output(B256::with_last_byte(1));
        call.elapsed
    };
    let cold = call(&mut ctx);
    let warm = call(&mut ctx);
    SystemRuntime::reset_cached_runtimes();
    let after_reset = call(&mut ctx);
    println!(
        "bls pairing tx (2 pairs): cold {:.3} s, warm {:.3} s, after reset {:.3} s",
        cold.as_secs_f64(),
        warm.as_secs_f64(),
        after_reset.as_secs_f64()
    );
}

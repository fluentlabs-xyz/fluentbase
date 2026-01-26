//! Unit tests for the EIP-2935 history storage contract.
//!
//! The contract has two entry paths:
//! - **submit**: if the caller is `SYSTEM_ADDRESS`, it stores the parent hash for `block_number - 1`
//!   into a ring buffer with size `EIP2935_HISTORY_SERVE_WINDOW`.
//! - **retrieve**: for any other caller, it interprets the 32-byte calldata as a big-endian block
//!   number and returns the stored hash for that block (if the request is within the serve window).
//!
//! These tests exercise:
//! - calldata validation (must be exactly 32 bytes)
//! - boundary conditions for the serve window
//! - caller gating (`SYSTEM_ADDRESS` vs user)
//! - ring-buffer wraparound and off-by-one errors
//!
//! The `HostTestingContext` is treated as the chain state. Each call clones the context, sets
//! caller/block number/input, runs the entrypoint, and then reads the produced output.

use crate::entrypoint;
use fluentbase_sdk::{Address, Bytes, ExitCode, EIP2935_HISTORY_SERVE_WINDOW, SYSTEM_ADDRESS};
use fluentbase_testing::TestingContextImpl;

const USER_ADDRESS: Address = Address::repeat_byte(0x11);

/// Gas limit used for each test call.
///
/// The exact value is not important for correctness tests; it just needs to be high enough so
/// that storage writes/reads do not exhaust gas.
const GAS_LIMIT: u64 = 10_000_000;

// ---- Helpers ----

fn u256_be_u64(n: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&n.to_be_bytes());
    out
}

/// Deterministic 32-byte value used as a stand-in for a block hash.
///
/// The contract only cares about storing and returning bytes; cryptographic properties are not
/// required for these unit tests. Keeping the generator dependency-free also makes it easier to
/// run tests in minimal environments.
fn pseudo_hash(tag: u8, n: u64) -> [u8; 32] {
    // Simple, stable, dependency-free mixer.
    let mut out = [0u8; 32];
    out[0] = tag;
    out[1..9].copy_from_slice(&n.to_be_bytes());
    // Diffuse a bit.
    for i in 0..32 {
        out[i] ^= (i as u8).wrapping_mul(31).wrapping_add(tag);
        out[i] = out[i].rotate_left((i % 8) as u32);
    }
    out
}

/// Maps an on-chain block number `k` to the value that must be returned by `retrieve(k)`.
///
/// EIP-2935 stores the parent hash for `block_number - 1` during a `submit` call made by the
/// system address. Therefore, to make block `k` return `hash_for_block(k)`, the test must call
/// `submit(hash_for_block(k))` at block `k + 1`.
fn hash_for_block(k: u64) -> [u8; 32] {
    pseudo_hash(0xA5, k)
}

/// Execute one precompile call in a controlled context.
///
/// This helper does **not** mutate the provided `sdk` in-place. Instead, it clones it,
/// configures the call parameters (input/caller/block number), runs the entrypoint, and then
/// extracts the output.
fn exec_as(
    sdk: &mut TestingContextImpl,
    sender: Address,
    block_number: u64,
    input: &[u8],
) -> Result<Bytes, ExitCode> {
    let gas_limit = GAS_LIMIT;

    // `HostTestingContext` is cheap to clone and is expected to share the underlying state
    // (storage, output buffer) via interior mutability. Each call configures a fresh view of the
    // context (caller / input / block number) and then executes the contract.
    let mut call_ctx = sdk
        .clone()
        .with_input(Bytes::copy_from_slice(input))
        .with_caller(sender)
        .with_block_number(block_number)
        .with_gas_limit(gas_limit);

    // `entrypoint` may take the context by value. We keep `call_ctx` around to read the produced
    // output after execution.
    entrypoint(&mut call_ctx)?;
    Ok(call_ctx.take_output().into())
}

/// Execute and require the call to succeed.
///
/// For this contract, both invalid input and out-of-window requests revert with `ExitCode::Panic`.
fn exec_ok(
    sdk: &mut TestingContextImpl,
    sender: Address,
    block_number: u64,
    input: &[u8],
) -> Bytes {
    exec_as(sdk, sender, block_number, input).expect("expected Ok, got revert/error")
}

fn exec_expect_revert(
    sdk: &mut TestingContextImpl,
    sender: Address,
    block_number: u64,
    input: &[u8],
) {
    let res = exec_as(sdk, sender, block_number, input);
    assert!(res.is_err(), "expected revert/error, got Ok");
}

fn set_at_block(sdk: &mut TestingContextImpl, block_number: u64, parent_hash: [u8; 32]) {
    // SYSTEM path: calldata is the hash to store; success produces empty output.
    let out = exec_ok(sdk, SYSTEM_ADDRESS, block_number, &parent_hash);
    assert!(
        out.is_empty(),
        "set() should return empty output; got {} bytes",
        out.len()
    );
}

fn get_at_block(sdk: &mut TestingContextImpl, block_number: u64, query_bn: u64) -> Bytes {
    let cd = u256_be_u64(query_bn);
    let out = exec_ok(sdk, USER_ADDRESS, block_number, &cd);
    assert_eq!(out.len(), 32, "get() should return 32 bytes");
    out
}

/// Populate history for blocks `[start_bn, start_bn + count - 1]`.
///
/// For each block `k` in the range, the test calls `submit(hash_for_block(k))` at block `k + 1`.
fn populate_history(sdk: &mut TestingContextImpl, start_bn: u64, count: u64) {
    for k in start_bn..(start_bn + count) {
        let at_block = k + 1;
        set_at_block(sdk, at_block, hash_for_block(k));
    }
}

// ---- Tests ----

#[test]
fn get_reverts_on_bad_calldata_length() {
    let mut sdk = TestingContextImpl::default();
    let current = 10_000;

    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &[]);
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &[0x01]);
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &[0u8; 31]);
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &[0u8; 33]);
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &[0u8; 64]);
}

#[test]
fn get_reverts_for_current_or_future_block() {
    let mut sdk = TestingContextImpl::default();
    let current = 20_000;

    // Fill enough history so in-range reads are meaningful.
    // Populate for blocks [current - W, current - 1].
    populate_history(
        &mut sdk,
        current - EIP2935_HISTORY_SERVE_WINDOW,
        EIP2935_HISTORY_SERVE_WINDOW,
    );

    // Querying current block number must revert (only hashes up to `current - 1` are served).
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &u256_be_u64(current));

    // Querying future must revert.
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &u256_be_u64(current + 1));
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &u256_be_u64(current + 123));
}

#[test]
fn get_reverts_for_too_old_block() {
    let mut sdk = TestingContextImpl::default();
    let current = 30_000;
    populate_history(
        &mut sdk,
        current - EIP2935_HISTORY_SERVE_WINDOW,
        EIP2935_HISTORY_SERVE_WINDOW,
    );

    // Oldest allowed is (current - W). One older must revert.
    let too_old = current - EIP2935_HISTORY_SERVE_WINDOW - 1;
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &u256_be_u64(too_old));
}

#[test]
fn get_succeeds_on_boundary_oldest_and_newest() {
    let mut sdk = TestingContextImpl::default();
    let current = 40_000;
    populate_history(
        &mut sdk,
        current - EIP2935_HISTORY_SERVE_WINDOW,
        EIP2935_HISTORY_SERVE_WINDOW,
    );

    let oldest = current - EIP2935_HISTORY_SERVE_WINDOW;
    let newest = current - 1;

    let got_oldest = get_at_block(&mut sdk, current, oldest);
    assert_eq!(got_oldest.as_ref(), &hash_for_block(oldest));

    let got_newest = get_at_block(&mut sdk, current, newest);
    assert_eq!(got_newest.as_ref(), &hash_for_block(newest));
}

#[test]
fn get_decodes_big_endian_correctly_via_behavior() {
    let mut sdk = TestingContextImpl::default();
    // The retrieve path expects a 32-byte big-endian block number.
    // We test this via behavior rather than by peeking into decoding internals.

    let current = 50_000;

    // We'll pick a target block and make sure it's populated.
    let target = current - 100;
    populate_history(&mut sdk, target, 200); // ensures [target..target+199] exist

    // Correct calldata: 32-byte big-endian for target.
    let good = u256_be_u64(target);
    let out_good = exec_ok(&mut sdk, USER_ADDRESS, current, &good);
    assert_eq!(out_good.as_ref(), &hash_for_block(target));

    // Craft a payload that looks like a number in the *front* of the 32-byte word.
    // If decoded as big-endian, this is astronomically large (and therefore out of range).
    let mut wrong = [0u8; 32];
    wrong[0..8].copy_from_slice(&target.to_le_bytes());

    // This should revert as out-of-range.
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &wrong);
}

#[test]
fn non_system_sender_never_triggers_set_path() {
    let mut sdk = TestingContextImpl::default();
    // If caller != SYSTEM, calldata is treated as retrieve(block_number).
    // A user cannot reach the submit path.
    let current = 60_000;

    // Populate a known value so we can check it's still correct afterwards.
    let known_block = current - 10;
    populate_history(&mut sdk, known_block, 20);

    // A user sends 32 bytes that look like a hash.
    // Under retrieve() rules, this is interpreted as a block number and should revert.
    let fake_hash = [0x42u8; 32];
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &fake_hash);

    // Ensure known history is still readable and correct.
    let got = get_at_block(&mut sdk, current, known_block);
    assert_eq!(got.as_ref(), &hash_for_block(known_block));
}

#[test]
fn bootstrap_unwritten_slots_return_zero_but_in_range() {
    let mut sdk = TestingContextImpl::default();
    // The ring buffer is populated gradually. Before a slot is written, its value is zero.
    // Within the serve window, such a slot should return `0x00..00` rather than reverting.

    let current = 70_000;

    // Populate only a small suffix of the window (the last 50 blocks).
    let start = current - 50;
    populate_history(&mut sdk, start, 50);

    // Query within allowed range but earlier than what we've populated.
    let oldest_allowed = current - EIP2935_HISTORY_SERVE_WINDOW;

    // Pick something in-range but not written: far away from [current-50..current-1].
    let in_range_unwritten = oldest_allowed + 123;

    // Should succeed and return zero (32 bytes of zero) in a fresh contract state.
    let got = get_at_block(&mut sdk, current, in_range_unwritten);
    assert_eq!(got.as_ref(), &[0u8; 32], "expected zero for unwritten slot");
}

#[test]
fn ring_wraparound_overwrites_expectedly() {
    let mut sdk = TestingContextImpl::default();

    let base = 100_000u64;

    // Write W + 10 parent hashes, which forces wraparound and overwrites the first 10 slots.
    let total = EIP2935_HISTORY_SERVE_WINDOW + 10;
    populate_history(&mut sdk, base, total);

    // Current block after all writes.
    let current = base + total;

    // Range at `current` is [current - W, current - 1] == [base + 10, base + W + 9].
    let oldest_allowed = current - EIP2935_HISTORY_SERVE_WINDOW;
    assert_eq!(oldest_allowed, base + 10);

    let newest = current - 1;

    // Newest in range should match.
    let got_newest = get_at_block(&mut sdk, current, newest);
    assert_eq!(got_newest.as_ref(), &hash_for_block(newest));

    // Too old must revert (outside window).
    exec_expect_revert(&mut sdk, USER_ADDRESS, current, &u256_be_u64(base));

    // Oldest allowed should match.
    let got_oldest = get_at_block(&mut sdk, current, oldest_allowed);
    assert_eq!(got_oldest.as_ref(), &hash_for_block(oldest_allowed));

    // Just-below-oldest must revert (off-by-one guard).
    exec_expect_revert(
        &mut sdk,
        USER_ADDRESS,
        current,
        &u256_be_u64(oldest_allowed - 1),
    );

    // Sanity check two distinct in-range blocks.
    let a = newest;
    let b = oldest_allowed;

    let got_a = get_at_block(&mut sdk, current, a);
    let got_b = get_at_block(&mut sdk, current, b);

    assert_eq!(got_a.as_ref(), &hash_for_block(a));
    assert_eq!(got_b.as_ref(), &hash_for_block(b));
    assert_ne!(
        got_a.as_ref(),
        got_b.as_ref(),
        "different blocks should have different hashes"
    );
}

#[test]
fn smoke_get_and_set_paths_produce_expected_outputs() {
    let mut sdk = TestingContextImpl::default();
    // This is a smoke test for the two entry paths. Gas accounting is tested elsewhere.
    let current = 120_000;

    // One set
    let out_set = exec_ok(
        &mut sdk,
        SYSTEM_ADDRESS,
        current,
        &hash_for_block(current - 1),
    );
    assert!(out_set.is_empty());

    // Populate enough so a get succeeds deterministically
    populate_history(&mut sdk, current - 100, 100);
    let out_get = exec_ok(&mut sdk, USER_ADDRESS, current, &u256_be_u64(current - 1));
    assert_eq!(out_get.as_ref(), &hash_for_block(current - 1));
}

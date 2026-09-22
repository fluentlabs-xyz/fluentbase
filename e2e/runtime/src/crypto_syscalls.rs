//! The precompile-level crypto syscalls from a contract's point of view.
//!
//! A wasm contract forwards its calldata to `_crypto_bls12381_pairing_check` and returns the
//! result byte. It must get the same answer as the native pairing check, pay the EIP-2537 gas
//! for the payload on top of its own execution, and get a clean error code for a bad payload.

use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{Address, Bytes, U256};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::{interpreter::gas::calculate_initial_tx_gas, primitives::hardfork::SpecId};

/// Reads the calldata into memory at 1024 (the raw input starts with the 1024-byte shared context
/// header), calls the syscall with it and writes `code(1 byte) || result(1 byte) || len(4 bytes)`.
const PAIRING_FORWARDER: &str = r#"(module
    (import "fluentbase_v1preview" "_input_size" (func $input_size (result i32)))
    (import "fluentbase_v1preview" "_read" (func $read (param i32 i32 i32)))
    (import "fluentbase_v1preview" "_write" (func $write (param i32 i32)))
    (import "fluentbase_v1preview" "_exit" (func $exit (param i32)))
    (import "fluentbase_v1preview" "_crypto_bls12381_pairing_check" (func $pairing (param i32 i32 i32) (result i32)))
    (memory (export "memory") 64)
    (func (export "main")
        (local $len i32)
        (local.set $len (i32.sub (call $input_size) (i32.const 1024)))
        (call $read (i32.const 1024) (i32.const 1024) (local.get $len))
        (i32.store8 (i32.const 0)
            (call $pairing (i32.const 1024) (local.get $len) (i32.const 1)))
        (i32.store (i32.const 2) (local.get $len))
        (call $write (i32.const 0) (i32.const 6))
        (call $exit (i32.const 0)))
    (func (export "deploy")))"#;

const PAIRING_BASE_GAS: u64 = 37_700;
const PAIRING_PAIR_GAS: u64 = 32_600;

fn deploy_forwarder(ctx: &mut EvmTestingContext) -> Address {
    let address = Address::repeat_byte(0xC7);
    let wasm = wat::parse_str(PAIRING_FORWARDER).expect("forwarder wat compiles");
    ctx.add_wasm_contract(address, &wasm);
    address
}

fn call(ctx: &mut EvmTestingContext, to: Address, input: Vec<u8>) -> (Vec<u8>, u64, bool) {
    let mut builder = TxBuilder::call(ctx, to)
        .caller(Address::ZERO)
        .input(Bytes::from(input))
        .gas_limit(30_000_000);
    builder.block.gas_limit = u64::MAX;
    let result = builder.exec();
    (
        result.output().map(|b| b.to_vec()).unwrap_or_default(),
        result.tx_gas_used(),
        result.is_success(),
    )
}

/// Unpadded `(G1, G2), (-G1, G2)` pairs whose product of pairings is one.
fn identity_pairs() -> Vec<u8> {
    let padded = super::bls12381::PAIRING_IDENTITY_PAIRS;
    let mut out = Vec::with_capacity(2 * (96 + 192));
    for pair in padded.chunks_exact(384) {
        // 6 padded field elements of 64 bytes, strip the 16 zero bytes each.
        for element in pair.chunks_exact(64) {
            out.extend_from_slice(&element[16..]);
        }
    }
    out
}

#[test]
fn contract_pairing_check_matches_native_and_pays_eip_gas() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(Address::ZERO, U256::from(u128::MAX));
    let forwarder = deploy_forwarder(&mut ctx);

    let pairs = identity_pairs();
    let (output, gas_used, success) = call(&mut ctx, forwarder, pairs.clone());
    assert!(success);
    assert_eq!(
        &output[..2],
        &[0u8, 1u8],
        "code 0, product of pairings is one: {output:?}"
    );

    // The same call with the second G2 point tampered fails the subgroup check on the host.
    let mut bad = pairs.clone();
    bad[96 + 5] ^= 0x01;
    let (output, _, success) = call(&mut ctx, forwarder, bad);
    assert!(success);
    assert_ne!(output[0], 0, "a bad point yields a non-zero code");

    // A payload that is not a whole number of pairs is rejected before any work is done.
    let (output, _, success) = call(&mut ctx, forwarder, pairs[..pairs.len() - 1].to_vec());
    assert!(success);
    assert_eq!(output[0], 1, "InvalidInput");

    // Gas: the contract pays the EIP-2537 price of two pairs on top of its own execution. The
    // remainder (reading input, the syscall entry, writing output) is a few thousand gas.
    let (_, gas_used_one_pair, _) = call(&mut ctx, forwarder, pairs[..288].to_vec());
    let intrinsic_two =
        calculate_initial_tx_gas(SpecId::PRAGUE, &pairs, false, 0, 0, 0).initial_total_gas;
    let intrinsic_one =
        calculate_initial_tx_gas(SpecId::PRAGUE, &pairs[..288], false, 0, 0, 0).initial_total_gas;
    let exec_two = gas_used - intrinsic_two;
    let exec_one = gas_used_one_pair - intrinsic_one;
    let precompile_two = PAIRING_BASE_GAS + 2 * PAIRING_PAIR_GAS;
    let precompile_one = PAIRING_BASE_GAS + PAIRING_PAIR_GAS;
    assert!(
        exec_two >= precompile_two,
        "two pairs: {exec_two} < {precompile_two}"
    );
    assert!(
        exec_one >= precompile_one,
        "one pair: {exec_one} < {precompile_one}"
    );
    assert!(
        exec_two - precompile_two < 20_000,
        "overhead beyond the EIP price: {}",
        exec_two - precompile_two
    );
    // The difference between the two calls is one pair's price plus the extra input bytes.
    let delta = exec_two - exec_one;
    assert!(
        delta >= PAIRING_PAIR_GAS && delta < PAIRING_PAIR_GAS + 2_000,
        "per-pair delta {delta}"
    );
}

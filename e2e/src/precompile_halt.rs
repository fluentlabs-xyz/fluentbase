//! Malformed system precompile inputs must halt and burn the whole gas supplied to the frame,
//! exactly like a halted precompile does in native REVM.
//!
//! The wrappers only charge gas on their success path (`sync_evm_gas`), and the non
//! engine-metered ones report almost no consumed fuel when they reject their input, so without a
//! central rule the caller would get the supplied gas back.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    address, Address, Bytes, PRECOMPILE_BIG_MODEXP, PRECOMPILE_BLAKE2F,
    PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BN256_ADD, PRECOMPILE_KZG_POINT_EVALUATION,
    PRECOMPILE_RIPEMD160,
};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;

const CALLER_ADDRESS: Address = address!("1234121212121212121212121212121212121234");

/// Enough gas for any of the precompiles below to run to completion on a well-formed input.
const GAS_LIMIT: u64 = 1_000_000;

sol! {
    function callExternal(address target, bytes calldata data) external returns (bool success, bytes memory result);
}

/// 213-byte BLAKE2F input with a final block indicator flag of `2` (only `0` and `1` are valid).
fn blake2f_malformed_flag() -> Bytes {
    hex!("0000000c48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b61626300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000002").into()
}

/// MODEXP header declaring a 2048-byte base, above the EIP-7823 input size limit of 1024.
fn modexp_oversized_base() -> Bytes {
    let mut input = [0u8; 96];
    input[30..32].copy_from_slice(&2048u16.to_be_bytes());
    input[63] = 1; // exponent length
    input[95] = 1; // modulus length
    input.into()
}

/// KZG point evaluation expects exactly 192 bytes.
fn kzg_short_input() -> Bytes {
    Bytes::from(vec![0u8; 100])
}

/// BLS12-381 G1 addition expects exactly 256 bytes.
fn bls12381_g1_add_short_input() -> Bytes {
    Bytes::from(vec![0u8; 10])
}

/// BN256 addition operands `(1, 1)` and `(0, 0)`; the first point is not on the curve.
fn bn256_add_point_not_on_curve() -> Bytes {
    hex!("0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").into()
}

/// Every malformed input below, when called directly, must halt and leave no gas behind.
fn assert_direct_call_burns_all_gas(name: &str, precompile: Address, input: Bytes, gas_limit: u64) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let result = ctx.call_evm_tx(CALLER_ADDRESS, precompile, input, Some(gas_limit), None);
    assert!(
        result.is_halt(),
        "{name}: malformed input must halt, got {result:?}"
    );
    assert_eq!(
        result.tx_gas_used(),
        gas_limit,
        "{name}: halted precompile must spend the entire gas limit"
    );
}

/// The same call made from a contract must fail without returning the forwarded gas either.
/// The caller keeps only the 1/64 the EVM withholds from a nested call.
fn assert_nested_call_burns_forwarded_gas(name: &str, precompile: Address, input: Bytes) {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let caller = ctx.deploy_evm_tx(
        CALLER_ADDRESS,
        hex::decode(include_bytes!("../assets/Caller.bin"))
            .unwrap()
            .into(),
    );

    let call_input = callExternalCall {
        target: precompile,
        data: input,
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        CALLER_ADDRESS,
        caller,
        call_input.into(),
        Some(GAS_LIMIT),
        None,
    );

    assert!(
        result.is_success(),
        "{name}: the caller itself must survive the failed sub-call, got {result:?}"
    );
    let decoded = callExternalCall::abi_decode_returns_validate(result.output().unwrap()).unwrap();
    assert!(
        !decoded.success,
        "{name}: nested call to a malformed precompile must fail"
    );
    assert!(
        decoded.result.is_empty(),
        "{name}: a halted precompile must not return any data"
    );

    // A nested call forwards 63/64 of the gas left at the CALL, and a halt burns all of it, so
    // the transaction must end up spending nearly its whole limit.
    let gas_used = result.tx_gas_used();
    assert!(
        gas_used * 100 >= GAS_LIMIT * 90,
        "{name}: forwarded gas was refunded, used only {gas_used} of {GAS_LIMIT}"
    );
}

#[test]
fn test_blake2f_malformed_input_burns_gas() {
    assert_direct_call_burns_all_gas(
        "blake2f",
        PRECOMPILE_BLAKE2F,
        blake2f_malformed_flag(),
        GAS_LIMIT,
    );
    assert_nested_call_burns_forwarded_gas("blake2f", PRECOMPILE_BLAKE2F, blake2f_malformed_flag());
}

#[test]
fn test_modexp_malformed_input_burns_gas() {
    assert_direct_call_burns_all_gas(
        "modexp",
        PRECOMPILE_BIG_MODEXP,
        modexp_oversized_base(),
        GAS_LIMIT,
    );
    assert_nested_call_burns_forwarded_gas(
        "modexp",
        PRECOMPILE_BIG_MODEXP,
        modexp_oversized_base(),
    );
}

#[test]
fn test_kzg_malformed_input_burns_gas() {
    assert_direct_call_burns_all_gas(
        "kzg",
        PRECOMPILE_KZG_POINT_EVALUATION,
        kzg_short_input(),
        GAS_LIMIT,
    );
    assert_nested_call_burns_forwarded_gas(
        "kzg",
        PRECOMPILE_KZG_POINT_EVALUATION,
        kzg_short_input(),
    );
}

#[test]
fn test_bls12381_malformed_input_burns_gas() {
    assert_direct_call_burns_all_gas(
        "bls12381-g1-add",
        PRECOMPILE_BLS12_381_G1_ADD,
        bls12381_g1_add_short_input(),
        GAS_LIMIT,
    );
    assert_nested_call_burns_forwarded_gas(
        "bls12381-g1-add",
        PRECOMPILE_BLS12_381_G1_ADD,
        bls12381_g1_add_short_input(),
    );
}

#[test]
fn test_bn256_invalid_point_burns_gas_after_precharge() {
    // BN256 precharges its flat cost before validating the operands, so the precharge must not
    // leave the rest of the frame's gas recoverable.
    assert_direct_call_burns_all_gas(
        "bn256-add",
        PRECOMPILE_BN256_ADD,
        bn256_add_point_not_on_curve(),
        GAS_LIMIT,
    );
    assert_nested_call_burns_forwarded_gas(
        "bn256-add",
        PRECOMPILE_BN256_ADD,
        bn256_add_point_not_on_curve(),
    );
}

#[test]
fn test_ripemd160_out_of_gas_burns_gas() {
    // RIPEMD160 accepts any input, so its only exceptional path is running out of gas: 21_000 is
    // the intrinsic cost of the transaction, leaving 200 gas for a call that needs 600.
    assert_direct_call_burns_all_gas("ripemd160", PRECOMPILE_RIPEMD160, Bytes::new(), 21_200);
}

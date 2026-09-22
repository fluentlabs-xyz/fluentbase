//! Throughput survey of the guest precompiles: how many gas per second each one sustains on
//! this build, measured through real transactions. Ignored by default; run with
//! `cargo test -p fluentbase-e2e --release precompile_throughput_survey -- --ignored --nocapture`.
//!
//! Two shapes are measured. Fixed-cost precompiles are called once per transaction, 100 times,
//! and the per-call time is the per-transaction time minus a plain-transfer baseline. Size-priced
//! precompiles get one large input. Gas is the EIP schedule for that input; anything far below
//! ~50 Mgas/s cannot fill a 50M-gas block inside a 1 s slot.

use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{
    Address, Bytes, PRECOMPILE_BIG_MODEXP, PRECOMPILE_BLAKE2F, PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD, PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR, PRECOMPILE_EIP7951,
    PRECOMPILE_IDENTITY, PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER, PRECOMPILE_SHA256, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use std::time::Instant;

const BN256_PAIR_2: [u8; 384] = hex!(
    "
    000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000
    00000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2
    1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395
    bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa
    000000000000000000000000000000000000000000000000000000000000000130644e72e131a029b85045b68181585d
    97816a916871ca8d3c208c16d87cfd45198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2
    1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395
    bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa
"
);
const BN256_MUL: [u8; 96] = hex!(
    "
    000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000
    0000000000000000000000000000000230644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd46
"
);
const BN256_ADD: [u8; 128] = hex!(
    "
    000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000
    000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000001
    0000000000000000000000000000000000000000000000000000000000000002
"
);
const KZG_POINT_EVAL: [u8; 192] = hex!(
    "
    01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b73eda753299d7d483339d80809a1d805
    53bda402fffe5bfeffffffff000000001522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9
    8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7
    a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c
"
);
const ECRECOVER: [u8; 128] = hex!(
    "
    18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c00000000000000000000000000000000
    0000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75f
    eeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549
"
);
const P256_VERIFY: [u8; 160] = hex!(
    "
    4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d169
    31ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d60
    4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc8
    26d0f7c00181a5fb2ddf79ae00b4e10e
"
);
const BLAKE2F_4M_ROUNDS: [u8; 213] = hex!(
    "
    003d090048c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b
    8c68059b6bbd41fbabd9831f79217e1319cde05b61626300000000000000000000000000000000000000000000000000
    000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    000000000300000000000000000000000000000001
"
);
const MODEXP_512B: [u8; 1152] = hex!(
    "
    000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000
    000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000200
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
    7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7fffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
    fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd
"
);

/// EIP-2537 identity pairing pairs, reused from the BLS tests.
const BLS_PAIRS: &[u8] = &super::bls12381::PAIRING_IDENTITY_PAIRS;

fn context() -> EvmTestingContext {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(Address::ZERO, U256::from(u128::MAX));
    ctx
}

/// Runs `n` transactions to `to` with `input` and returns (seconds per tx, gas used per tx).
fn per_tx(ctx: &mut EvmTestingContext, to: Address, input: &[u8], n: u32) -> (f64, u64) {
    let mut gas = 0;
    let started = Instant::now();
    for _ in 0..n {
        let mut builder = TxBuilder::call(ctx, to)
            .caller(Address::ZERO)
            .input(Bytes::copy_from_slice(input))
            .gas_limit(60_000_000);
        builder.block.gas_limit = u64::MAX;
        let result = builder.exec();
        assert!(result.is_success(), "{to}: {result:?}");
        gas = result.tx_gas_used();
    }
    (started.elapsed().as_secs_f64() / n as f64, gas)
}

fn calldata_gas(input: &[u8]) -> u64 {
    input.iter().map(|b| if *b == 0 { 4 } else { 16 }).sum()
}

#[test]
#[ignore = "throughput survey; run manually with --ignored --nocapture"]
fn precompile_throughput_survey() {
    println!("backend: {}", super::bls12381::BACKEND);
    let mut ctx = context();
    // Warm every runtime once so the survey never pays a cold load.
    let warm: [(Address, &[u8]); 11] = [
        (PRECOMPILE_SECP256K1_RECOVER, &ECRECOVER),
        (PRECOMPILE_EIP7951, &P256_VERIFY),
        (PRECOMPILE_KZG_POINT_EVALUATION, &KZG_POINT_EVAL),
        (PRECOMPILE_BN256_ADD, &BN256_ADD),
        (PRECOMPILE_BN256_MUL, &BN256_MUL),
        (PRECOMPILE_BN256_PAIR, &BN256_PAIR_2),
        (PRECOMPILE_SHA256, &[0u8; 32]),
        (PRECOMPILE_IDENTITY, &[0u8; 32]),
        (PRECOMPILE_BIG_MODEXP, &MODEXP_512B),
        (PRECOMPILE_BLAKE2F, &BLAKE2F_4M_ROUNDS),
        (PRECOMPILE_RIPEMD160, &[0u8; 32]),
    ];
    for (to, input) in warm {
        per_tx(&mut ctx, to, input, 1);
    }
    let (baseline, _) = per_tx(&mut ctx, Address::repeat_byte(0x11), &[], 100);
    println!("plain transfer baseline: {:.3} ms per tx", baseline * 1e3);
    println!(
        "{:<28} {:>10} {:>12} {:>10}",
        "fixed-cost precompile", "spec gas", "ms per call", "Mgas/s"
    );
    let fixed: [(&str, Address, &[u8], u64); 8] = [
        ("ecrecover", PRECOMPILE_SECP256K1_RECOVER, &ECRECOVER, 3_000),
        (
            "p256verify (EIP-7951)",
            PRECOMPILE_EIP7951,
            &P256_VERIFY,
            6_900,
        ),
        (
            "kzg point evaluation",
            PRECOMPILE_KZG_POINT_EVALUATION,
            &KZG_POINT_EVAL,
            50_000,
        ),
        ("bn256 add", PRECOMPILE_BN256_ADD, &BN256_ADD, 150),
        ("bn256 mul", PRECOMPILE_BN256_MUL, &BN256_MUL, 6_000),
        (
            "bn256 pairing, 2 pairs",
            PRECOMPILE_BN256_PAIR,
            &BN256_PAIR_2,
            45_000 + 2 * 34_000,
        ),
        ("sha256, 32 bytes", PRECOMPILE_SHA256, &[0u8; 32], 72),
        ("identity, 32 bytes", PRECOMPILE_IDENTITY, &[0u8; 32], 18),
    ];
    for (name, to, input, spec_gas) in fixed {
        let (secs, _) = per_tx(&mut ctx, to, input, 100);
        let call = (secs - baseline).max(1e-6);
        println!(
            "{name:<28} {spec_gas:>10} {:>12.3} {:>10.1}",
            call * 1e3,
            spec_gas as f64 / call / 1e6
        );
    }
    println!(
        "{:<28} {:>10} {:>12} {:>10}",
        "size-priced, one call", "prec. gas", "seconds", "Mgas/s"
    );
    let bls_pairs = 128;
    let bls_input: Vec<u8> = BLS_PAIRS.repeat(bls_pairs / 2);
    let bn_pairs = 60;
    let bn_input: Vec<u8> = BN256_PAIR_2.repeat(bn_pairs / 2);
    let big = vec![0x5au8; 128 * 1024];
    let words = |n: usize| ((n + 31) / 32) as u64;
    let sized: [(&str, Address, &[u8], u64); 7] = [
        (
            "bls pairing, 128 pairs",
            PRECOMPILE_BLS12_381_PAIRING,
            &bls_input,
            37_700 + 32_600 * bls_pairs as u64,
        ),
        (
            "bn256 pairing, 60 pairs",
            PRECOMPILE_BN256_PAIR,
            &bn_input,
            45_000 + 34_000 * bn_pairs as u64,
        ),
        // Osaka (EIP-7883) price of this input, taken from the transaction itself.
        (
            "modexp 512B base/mod, 256-bit exp",
            PRECOMPILE_BIG_MODEXP,
            &MODEXP_512B,
            0,
        ),
        (
            "blake2f, 4M rounds",
            PRECOMPILE_BLAKE2F,
            &BLAKE2F_4M_ROUNDS,
            4_000_000,
        ),
        (
            "sha256, 128 KiB",
            PRECOMPILE_SHA256,
            &big,
            60 + 12 * words(big.len()),
        ),
        (
            "ripemd160, 128 KiB",
            PRECOMPILE_RIPEMD160,
            &big,
            600 + 120 * words(big.len()),
        ),
        (
            "identity, 128 KiB",
            PRECOMPILE_IDENTITY,
            &big,
            15 + 3 * words(big.len()),
        ),
    ];
    for (name, to, input, spec_gas) in sized {
        let (secs, gas) = per_tx(&mut ctx, to, input, 1);
        // The EIP-7623 floor can dominate `gas` for large inputs, so use the schedule where known.
        let precompile_gas = if spec_gas > 0 {
            spec_gas
        } else {
            gas.saturating_sub(21_000 + calldata_gas(input))
        };
        let exec = (secs - baseline).max(1e-6);
        println!(
            "{name:<28} {precompile_gas:>10} {:>12.3} {:>10.1}",
            exec,
            precompile_gas as f64 / exec / 1e6
        );
    }
}

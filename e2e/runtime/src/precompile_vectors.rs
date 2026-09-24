//! Differential tests for every Ethereum precompile address.
//!
//! Each vector runs twice: directly through the `revm::precompile` function and as a transaction
//! to the precompile address, which the EVM dispatches through `RwasmPrecompiles`. Both must
//! produce the same output and the same total gas, or both must fail. The BLS12-381 vectors are
//! the EIP-2537 suites in `assets/bls12381`; the others are the vectors from the unit tests of
//! the former rWASM guests plus a few constructed inputs.
//!
//! The same suite passed against the genesis rWASM guests before the native provider replaced
//! them, which is what made that replacement gas- and output-neutral.

use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{
    Address, Bytes, PRECOMPILE_BIG_MODEXP, PRECOMPILE_BLAKE2F, PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD, PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR, PRECOMPILE_EIP7951,
    PRECOMPILE_IDENTITY, PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER, PRECOMPILE_SHA256, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::{
    interpreter::gas::calculate_initial_tx_gas,
    precompile::{
        blake2, bls12_381, bn254, hash, identity, kzg_point_evaluation, modexp, secp256k1,
        secp256r1, EthPrecompileResult,
    },
    primitives::hardfork::SpecId,
};
use serde::Deserialize;

const GAS_LIMIT: u64 = 30_000_000;

type Native = Box<dyn Fn(&[u8], u64) -> EthPrecompileResult>;

struct Case {
    name: String,
    address: Address,
    input: Vec<u8>,
    native: Native,
}

fn native(f: fn(&[u8], u64) -> EthPrecompileResult) -> Native {
    Box::new(f)
}

fn context() -> EvmTestingContext {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(Address::ZERO, U256::from(u128::MAX));
    ctx
}

/// Executes the case natively and as a transaction and compares the two.
fn run_case(ctx: &mut EvmTestingContext, case: &Case) {
    let native = (case.native)(&case.input, GAS_LIMIT);
    let mut builder = TxBuilder::call(ctx, case.address)
        .caller(Address::ZERO)
        .input(Bytes::from(case.input.clone()))
        .gas_limit(GAS_LIMIT);
    builder.block.gas_limit = u64::MAX;
    let result = builder.exec();
    match native {
        Ok(out) => {
            assert!(
                result.is_success(),
                "{}: the transaction failed where the precompile function succeeded: {result:?}",
                case.name
            );
            assert_eq!(
                result.output().map(|b| b.as_ref()),
                Some(out.bytes.as_ref()),
                "{}: output differs",
                case.name
            );
            let initial = calculate_initial_tx_gas(SpecId::PRAGUE, &case.input, false, 0, 0, 0);
            let expected_gas = (initial.initial_total_gas + out.gas_used).max(initial.floor_gas);
            assert_eq!(
                result.tx_gas_used(),
                expected_gas,
                "{}: gas differs (native precompile gas {})",
                case.name,
                out.gas_used
            );
        }
        Err(err) => {
            assert!(
                !result.is_success(),
                "{}: the transaction succeeded where the precompile function failed with {err:?}: {result:?}",
                case.name
            );
            assert_eq!(
                result.tx_gas_used(),
                GAS_LIMIT,
                "{}: a failing precompile consumes the whole gas limit",
                case.name
            );
        }
    }
}

fn run_cases(cases: Vec<Case>) {
    let mut ctx = context();
    for case in &cases {
        run_case(&mut ctx, case);
    }
}

/// Must match the JSON keys exactly.
#[derive(Deserialize)]
struct BlsVector {
    #[serde(rename = "Input")]
    input: String,
    #[serde(rename = "Name")]
    name: String,
}

fn decode_hex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("valid hex")
}

fn bls_cases(
    json: &str,
    address: Address,
    native_fn: fn(&[u8], u64) -> EthPrecompileResult,
) -> Vec<Case> {
    let vectors: Vec<BlsVector> = serde_json::from_str(json).expect("valid BLS test JSON");
    vectors
        .into_iter()
        .map(|v| Case {
            name: v.name,
            address,
            input: decode_hex(&v.input),
            native: native(native_fn),
        })
        .collect()
}

macro_rules! bls_suite {
    ($test:ident, $file:literal, $address:expr, $native:path) => {
        #[test]
        fn $test() {
            run_cases(bls_cases(
                include_str!(concat!("../assets/bls12381/", $file, ".json")),
                $address,
                $native,
            ));
        }
    };
}

bls_suite!(
    bls_g1_add_vectors,
    "add_G1_bls",
    PRECOMPILE_BLS12_381_G1_ADD,
    bls12_381::g1_add::g1_add
);
bls_suite!(
    bls_g1_add_failures,
    "fail-add_G1_bls",
    PRECOMPILE_BLS12_381_G1_ADD,
    bls12_381::g1_add::g1_add
);
bls_suite!(
    bls_g2_add_vectors,
    "add_G2_bls",
    PRECOMPILE_BLS12_381_G2_ADD,
    bls12_381::g2_add::g2_add
);
bls_suite!(
    bls_g2_add_failures,
    "fail-add_G2_bls",
    PRECOMPILE_BLS12_381_G2_ADD,
    bls12_381::g2_add::g2_add
);
bls_suite!(
    bls_g1_msm_vectors,
    "msm_G1_bls",
    PRECOMPILE_BLS12_381_G1_MSM,
    bls12_381::g1_msm::g1_msm
);
bls_suite!(
    bls_g1_mul_vectors,
    "mul_G1_bls",
    PRECOMPILE_BLS12_381_G1_MSM,
    bls12_381::g1_msm::g1_msm
);
bls_suite!(
    bls_g1_msm_failures,
    "fail-msm_G1_bls",
    PRECOMPILE_BLS12_381_G1_MSM,
    bls12_381::g1_msm::g1_msm
);
bls_suite!(
    bls_g1_mul_failures,
    "fail-mul_G1_bls",
    PRECOMPILE_BLS12_381_G1_MSM,
    bls12_381::g1_msm::g1_msm
);
bls_suite!(
    bls_g2_msm_vectors,
    "msm_G2_bls",
    PRECOMPILE_BLS12_381_G2_MSM,
    bls12_381::g2_msm::g2_msm
);
bls_suite!(
    bls_g2_mul_vectors,
    "mul_G2_bls",
    PRECOMPILE_BLS12_381_G2_MSM,
    bls12_381::g2_msm::g2_msm
);
bls_suite!(
    bls_g2_msm_failures,
    "fail-msm_G2_bls",
    PRECOMPILE_BLS12_381_G2_MSM,
    bls12_381::g2_msm::g2_msm
);
bls_suite!(
    bls_g2_mul_failures,
    "fail-mul_G2_bls",
    PRECOMPILE_BLS12_381_G2_MSM,
    bls12_381::g2_msm::g2_msm
);
bls_suite!(
    bls_pairing_vectors,
    "pairing_check_bls",
    PRECOMPILE_BLS12_381_PAIRING,
    bls12_381::pairing::pairing
);
bls_suite!(
    bls_pairing_failures,
    "fail-pairing_check_bls",
    PRECOMPILE_BLS12_381_PAIRING,
    bls12_381::pairing::pairing
);
bls_suite!(
    bls_map_fp_to_g1_vectors,
    "map_fp_to_G1_bls",
    PRECOMPILE_BLS12_381_MAP_G1,
    bls12_381::map_fp_to_g1::map_fp_to_g1
);
bls_suite!(
    bls_map_fp_to_g1_failures,
    "fail-map_fp_to_G1_bls",
    PRECOMPILE_BLS12_381_MAP_G1,
    bls12_381::map_fp_to_g1::map_fp_to_g1
);
bls_suite!(
    bls_map_fp2_to_g2_vectors,
    "map_fp2_to_G2_bls",
    PRECOMPILE_BLS12_381_MAP_G2,
    bls12_381::map_fp2_to_g2::map_fp2_to_g2
);
bls_suite!(
    bls_map_fp2_to_g2_failures,
    "fail-map_fp2_to_G2_bls",
    PRECOMPILE_BLS12_381_MAP_G2,
    bls12_381::map_fp2_to_g2::map_fp2_to_g2
);

fn case(name: &str, address: Address, input: &[u8], native_fn: Native) -> Case {
    Case {
        name: name.to_string(),
        address,
        input: input.to_vec(),
        native: native_fn,
    }
}

#[test]
fn ecrecover_vectors() {
    let valid = hex!("18c547e4f7b0f325ad1e56f57e26c745b09a3e503d86e00e5255ff7f715d3d1c000000000000000000000000000000000000000000000000000000000000001c73b1693892219d736caba55bdb67216e485557ea6b6af75f37096c9aa6a5a75feeb940b1d03b21e36b0e47e79769f095fe2ab855bd91e3a38756b7d75a9c4549");
    let mut bad_v = valid;
    bad_v[32] = 0x10;
    let mut bad_sig = valid;
    bad_sig[64] ^= 0xff;
    run_cases(vec![
        case(
            "ecrecover valid",
            PRECOMPILE_SECP256K1_RECOVER,
            &valid,
            native(secp256k1::ec_recover_run),
        ),
        case(
            "ecrecover invalid v",
            PRECOMPILE_SECP256K1_RECOVER,
            &bad_v,
            native(secp256k1::ec_recover_run),
        ),
        case(
            "ecrecover invalid r",
            PRECOMPILE_SECP256K1_RECOVER,
            &bad_sig,
            native(secp256k1::ec_recover_run),
        ),
        case(
            "ecrecover short input",
            PRECOMPILE_SECP256K1_RECOVER,
            &valid[..100],
            native(secp256k1::ec_recover_run),
        ),
        case(
            "ecrecover empty input",
            PRECOMPILE_SECP256K1_RECOVER,
            &[],
            native(secp256k1::ec_recover_run),
        ),
    ]);
}

#[test]
fn p256verify_vectors() {
    let valid = hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e");
    let mut invalid = valid;
    invalid[0] ^= 0x01;
    run_cases(vec![
        case(
            "p256verify valid",
            PRECOMPILE_EIP7951,
            &valid,
            native(secp256r1::p256_verify_osaka),
        ),
        case(
            "p256verify invalid",
            PRECOMPILE_EIP7951,
            &invalid,
            native(secp256r1::p256_verify_osaka),
        ),
        case(
            "p256verify short input",
            PRECOMPILE_EIP7951,
            &valid[..159],
            native(secp256r1::p256_verify_osaka),
        ),
    ]);
}

#[test]
fn kzg_point_evaluation_vectors() {
    // c-kzg verify_kzg_proof_case_correct_proof_31ebd010e6098750, versioned hash derived.
    let commitment = hex!("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7");
    let z = hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000");
    let y = hex!("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9");
    let proof = hex!("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c");
    let mut versioned_hash: [u8; 32] = {
        use sha2::{Digest, Sha256};
        Sha256::digest(commitment).into()
    };
    versioned_hash[0] = 0x01;
    let mut valid = Vec::new();
    valid.extend_from_slice(&versioned_hash);
    valid.extend_from_slice(&z);
    valid.extend_from_slice(&y);
    valid.extend_from_slice(&commitment);
    valid.extend_from_slice(&proof);
    let mut bad_proof = valid.clone();
    bad_proof[190] ^= 0x01;
    let mut bad_hash = valid.clone();
    bad_hash[5] ^= 0x01;
    run_cases(vec![
        case(
            "kzg valid",
            PRECOMPILE_KZG_POINT_EVALUATION,
            &valid,
            native(kzg_point_evaluation::run),
        ),
        case(
            "kzg bad proof",
            PRECOMPILE_KZG_POINT_EVALUATION,
            &bad_proof,
            native(kzg_point_evaluation::run),
        ),
        case(
            "kzg bad versioned hash",
            PRECOMPILE_KZG_POINT_EVALUATION,
            &bad_hash,
            native(kzg_point_evaluation::run),
        ),
        case(
            "kzg short input",
            PRECOMPILE_KZG_POINT_EVALUATION,
            &valid[..100],
            native(kzg_point_evaluation::run),
        ),
    ]);
}

#[test]
fn bn256_vectors() {
    use bn254::{
        add::ISTANBUL_ADD_GAS_COST,
        mul::ISTANBUL_MUL_GAS_COST,
        pair::{ISTANBUL_PAIR_BASE, ISTANBUL_PAIR_PER_POINT},
    };
    let add: fn(&[u8], u64) -> EthPrecompileResult =
        |input, gas| bn254::run_add(input, ISTANBUL_ADD_GAS_COST, gas);
    let mul: fn(&[u8], u64) -> EthPrecompileResult =
        |input, gas| bn254::run_mul(input, ISTANBUL_MUL_GAS_COST, gas);
    let pair: fn(&[u8], u64) -> EthPrecompileResult =
        |input, gas| bn254::run_pair(input, ISTANBUL_PAIR_PER_POINT, ISTANBUL_PAIR_BASE, gas);

    // Generator, 2G and -G on bn254 G1, big-endian.
    let g1 = hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002");
    let two_g1 = hex!("030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd315ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4");
    let neg_g1 = hex!("000000000000000000000000000000000000000000000000000000000000000130644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd45");
    let g2 = hex!("198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa");
    let chfast1 = hex!("18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f3726607c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7");
    let mut add_gg = Vec::new();
    add_gg.extend_from_slice(&g1);
    add_gg.extend_from_slice(&g1);
    let mut add_g_neg = Vec::new();
    add_g_neg.extend_from_slice(&g1);
    add_g_neg.extend_from_slice(&neg_g1);
    let mut add_g_zero = Vec::new();
    add_g_zero.extend_from_slice(&g1);
    add_g_zero.extend_from_slice(&[0u8; 64]);
    let mut add_g_2g = Vec::new();
    add_g_2g.extend_from_slice(&g1);
    add_g_2g.extend_from_slice(&two_g1);
    let mut not_on_curve = add_gg.clone();
    not_on_curve[63] = 3;
    let mut mul_g2 = Vec::new();
    mul_g2.extend_from_slice(&g1);
    mul_g2.extend_from_slice(&hex!(
        "0000000000000000000000000000000000000000000000000000000000000002"
    ));
    let mut mul_big = Vec::new();
    mul_big.extend_from_slice(&two_g1);
    mul_big.extend_from_slice(&[0xffu8; 32]);
    // e(G1, G2) * e(-G1, G2) = 1 and e(G1, G2) != 1.
    let mut pair_ok = Vec::new();
    pair_ok.extend_from_slice(&g1);
    pair_ok.extend_from_slice(&g2);
    pair_ok.extend_from_slice(&neg_g1);
    pair_ok.extend_from_slice(&g2);
    let mut pair_no = Vec::new();
    pair_no.extend_from_slice(&g1);
    pair_no.extend_from_slice(&g2);
    let mut pair_bad_len = pair_ok.clone();
    pair_bad_len.pop();
    run_cases(vec![
        case("bn256 add g+g", PRECOMPILE_BN256_ADD, &add_gg, native(add)),
        case(
            "bn256 add g+(-g)",
            PRECOMPILE_BN256_ADD,
            &add_g_neg,
            native(add),
        ),
        case(
            "bn256 add g+0",
            PRECOMPILE_BN256_ADD,
            &add_g_zero,
            native(add),
        ),
        case(
            "bn256 add g+2g",
            PRECOMPILE_BN256_ADD,
            &add_g_2g,
            native(add),
        ),
        case(
            "bn256 add chfast1",
            PRECOMPILE_BN256_ADD,
            &chfast1,
            native(add),
        ),
        case(
            "bn256 add short input",
            PRECOMPILE_BN256_ADD,
            &g1,
            native(add),
        ),
        case(
            "bn256 add empty input",
            PRECOMPILE_BN256_ADD,
            &[],
            native(add),
        ),
        case(
            "bn256 add not on curve",
            PRECOMPILE_BN256_ADD,
            &not_on_curve,
            native(add),
        ),
        case("bn256 mul g*2", PRECOMPILE_BN256_MUL, &mul_g2, native(mul)),
        case(
            "bn256 mul 2g*big",
            PRECOMPILE_BN256_MUL,
            &mul_big,
            native(mul),
        ),
        case(
            "bn256 mul short input",
            PRECOMPILE_BN256_MUL,
            &g1,
            native(mul),
        ),
        case(
            "bn256 pair identity product",
            PRECOMPILE_BN256_PAIR,
            &pair_ok,
            native(pair),
        ),
        case(
            "bn256 pair non-identity",
            PRECOMPILE_BN256_PAIR,
            &pair_no,
            native(pair),
        ),
        case("bn256 pair empty", PRECOMPILE_BN256_PAIR, &[], native(pair)),
        case(
            "bn256 pair bad length",
            PRECOMPILE_BN256_PAIR,
            &pair_bad_len,
            native(pair),
        ),
    ]);
}

#[test]
fn modexp_vectors() {
    let v1 = hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000002003fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2efffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");
    let v2 = hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000020fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2efffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f");
    // Large sizes, rejected by the EIP-7823 limit.
    let mut too_big = [0u8; 96];
    too_big[31] = 0x04; // base length 1024 + 1
    too_big[30] = 0x01;
    run_cases(vec![
        case(
            "modexp v1",
            PRECOMPILE_BIG_MODEXP,
            &v1,
            native(modexp::osaka_run),
        ),
        case(
            "modexp v2",
            PRECOMPILE_BIG_MODEXP,
            &v2,
            native(modexp::osaka_run),
        ),
        case(
            "modexp empty",
            PRECOMPILE_BIG_MODEXP,
            &[],
            native(modexp::osaka_run),
        ),
        case(
            "modexp over size limit",
            PRECOMPILE_BIG_MODEXP,
            &too_big,
            native(modexp::osaka_run),
        ),
    ]);
}

#[test]
fn blake2f_vectors() {
    let twelve_rounds = hex!("0000000c48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b61626300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000001");
    let mut zero_rounds = twelve_rounds;
    zero_rounds[..4].copy_from_slice(&[0, 0, 0, 0]);
    let mut bad_flag = twelve_rounds;
    bad_flag[212] = 2;
    run_cases(vec![
        case(
            "blake2f 12 rounds",
            PRECOMPILE_BLAKE2F,
            &twelve_rounds,
            native(blake2::run),
        ),
        case(
            "blake2f 0 rounds",
            PRECOMPILE_BLAKE2F,
            &zero_rounds,
            native(blake2::run),
        ),
        case(
            "blake2f bad final flag",
            PRECOMPILE_BLAKE2F,
            &bad_flag,
            native(blake2::run),
        ),
        case(
            "blake2f short input",
            PRECOMPILE_BLAKE2F,
            &twelve_rounds[..212],
            native(blake2::run),
        ),
    ]);
}

#[test]
fn hash_and_identity_vectors() {
    let inputs: [&[u8]; 4] = [b"", b"abc", &[0x5a; 100], &[0u8; 4096]];
    let mut cases = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        cases.push(case(
            &format!("sha256 #{i}"),
            PRECOMPILE_SHA256,
            input,
            native(hash::sha256_run),
        ));
        cases.push(case(
            &format!("ripemd160 #{i}"),
            PRECOMPILE_RIPEMD160,
            input,
            native(hash::ripemd160_run),
        ));
        cases.push(case(
            &format!("identity #{i}"),
            PRECOMPILE_IDENTITY,
            input,
            native(identity::identity_run),
        ));
    }
    run_cases(cases);
}

//! Reference tests for the crypto syscall handlers against independent implementations.
//!
//! Every `_impl` function is checked byte for byte against arkworks (bn254, bls12-381), k256
//! (secp256k1) and p256 (secp256r1), including the validation rules and the degenerate-input
//! behaviour inherited from SP1. The point is to pin the observable behaviour of the handlers so
//! their arithmetic can be replaced without changing a single result: these syscalls are
//! consensus surface for every hint and contract that imports them.

use super::*;
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{BigInteger, Field, PrimeField, UniformRand};
use ark_std::rand::{rngs::StdRng as ArkRng, SeedableRng as ArkSeedableRng};
use fluentbase_types::{
    ExitCode, BLS12381_FP_SIZE, BLS12381_G1_COMPRESSED_SIZE, BLS12381_G1_RAW_AFFINE_SIZE,
    BN254_FP_SIZE, BN254_G1_RAW_AFFINE_SIZE, SECP256K1_G1_COMPRESSED_SIZE,
    SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE, SECP256R1_G1_RAW_AFFINE_SIZE,
};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use num::BigUint;
use rand::{rngs::StdRng, Rng, RngCore, SeedableRng};
use sp1_curves::{
    params::FieldParameters,
    weierstrass::{
        bls12_381::Bls12381BaseField, bn254::Bn254BaseField, secp256k1::Secp256k1BaseField,
        secp256r1::Secp256r1BaseField,
    },
};

const ROUNDS: usize = 64;

fn rng() -> StdRng {
    StdRng::seed_from_u64(0x5ca1ab1e)
}

/// arkworks 0.5 samples through rand_core 0.6, so it gets its own seeded generator.
fn ark_rng() -> ArkRng {
    ArkRng::seed_from_u64(0x5ca1ab1e)
}

fn fe_to_le<F: PrimeField, const N: usize>(f: &F) -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = f.into_bigint().to_bytes_le();
    out[..bytes.len()].copy_from_slice(&bytes);
    out
}

fn le_to_fe<F: PrimeField>(bytes: &[u8]) -> F {
    F::from_le_bytes_mod_order(bytes)
}

fn biguint_to_le<const N: usize>(v: &BigUint) -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = v.to_bytes_le();
    assert!(bytes.len() <= N, "value does not fit");
    out[..bytes.len()].copy_from_slice(&bytes);
    out
}

/// Runs the fp1 add/sub/mul contract for one field against arkworks.
fn check_fp1<F: PrimeField, const N: usize>(
    modulus: &BigUint,
    add: impl Fn([u8; N], [u8; N]) -> Result<[u8; N], ExitCode>,
    sub: impl Fn([u8; N], [u8; N]) -> Result<[u8; N], ExitCode>,
    mul: impl Fn([u8; N], [u8; N]) -> Result<[u8; N], ExitCode>,
) {
    let mut rng = ark_rng();
    let mut cases: Vec<(F, F)> = (0..ROUNDS)
        .map(|_| (F::rand(&mut rng), F::rand(&mut rng)))
        .collect();
    let minus_one = -F::ONE;
    cases.extend([
        (F::ZERO, F::ZERO),
        (F::ONE, F::ZERO),
        (F::ZERO, F::ONE),
        (minus_one, minus_one),
        (minus_one, F::ONE),
        (F::ONE, minus_one),
    ]);
    for (a, b) in cases {
        let (x, y) = (fe_to_le::<F, N>(&a), fe_to_le::<F, N>(&b));
        assert_eq!(add(x, y).unwrap(), fe_to_le::<F, N>(&(a + b)), "add");
        assert_eq!(sub(x, y).unwrap(), fe_to_le::<F, N>(&(a - b)), "sub");
        assert_eq!(mul(x, y).unwrap(), fe_to_le::<F, N>(&(a * b)), "mul");
    }

    // Non-canonical inputs (>= modulus) are accepted and reduced, as the BigUint path does.
    let r = F::rand(&mut rng);
    let s = F::rand(&mut rng);
    let r_int = BigUint::from_bytes_le(&fe_to_le::<F, N>(&r));
    let s_int = BigUint::from_bytes_le(&fe_to_le::<F, N>(&s));
    let r_plus_p: [u8; N] = biguint_to_le(&(&r_int + modulus));
    let s_plus_p: [u8; N] = biguint_to_le(&(&s_int + modulus));
    let s_le = fe_to_le::<F, N>(&s);
    assert_eq!(add(r_plus_p, s_le).unwrap(), fe_to_le::<F, N>(&(r + s)));
    assert_eq!(add(r_plus_p, s_plus_p).unwrap(), fe_to_le::<F, N>(&(r + s)));
    assert_eq!(mul(r_plus_p, s_plus_p).unwrap(), fe_to_le::<F, N>(&(r * s)));
    assert_eq!(sub(r_plus_p, s_le).unwrap(), fe_to_le::<F, N>(&(r - s)));

    // Subtraction rejects `b > a + p` on the raw integers and accepts `b == a + p`.
    let zero = [0u8; N];
    let p_le: [u8; N] = biguint_to_le(modulus);
    let p_plus_one: [u8; N] = biguint_to_le(&(modulus + 1u32));
    assert_eq!(sub(zero, p_le).unwrap(), zero);
    assert_eq!(sub(zero, p_plus_one), Err(ExitCode::MalformedBuiltinParams));
    assert_eq!(sub(p_le, p_plus_one).unwrap(), fe_to_le::<F, N>(&-F::ONE));
}

/// Runs the fp2 add/sub/mul contract for one field against arkworks' `Fq2` (non-residue -1).
fn check_fp2<F: PrimeField, F2, const N: usize>(
    modulus: &BigUint,
    new_fp2: impl Fn(F, F) -> F2,
    coords: impl Fn(&F2) -> (F, F),
    add: impl Fn([u8; N], [u8; N], [u8; N], [u8; N]) -> Result<([u8; N], [u8; N]), ExitCode>,
    sub: impl Fn([u8; N], [u8; N], [u8; N], [u8; N]) -> Result<([u8; N], [u8; N]), ExitCode>,
    mul: impl Fn([u8; N], [u8; N], [u8; N], [u8; N]) -> Result<([u8; N], [u8; N]), ExitCode>,
) where
    F2: Field,
{
    let mut rng = ark_rng();
    let enc = |v: &F2| {
        let (c0, c1) = coords(v);
        (fe_to_le::<F, N>(&c0), fe_to_le::<F, N>(&c1))
    };
    for _ in 0..ROUNDS {
        let a = new_fp2(F::rand(&mut rng), F::rand(&mut rng));
        let b = new_fp2(F::rand(&mut rng), F::rand(&mut rng));
        let (a0, a1) = enc(&a);
        let (b0, b1) = enc(&b);
        assert_eq!(add(a0, a1, b0, b1).unwrap(), enc(&(a + b)), "fp2 add");
        assert_eq!(sub(a0, a1, b0, b1).unwrap(), enc(&(a - b)), "fp2 sub");
        assert_eq!(mul(a0, a1, b0, b1).unwrap(), enc(&(a * b)), "fp2 mul");
    }
    // Each limb of a subtraction is checked independently on the raw integers.
    let zero = [0u8; N];
    let p_plus_one: [u8; N] = biguint_to_le(&(modulus + 1u32));
    assert_eq!(
        sub(zero, zero, p_plus_one, zero),
        Err(ExitCode::MalformedBuiltinParams)
    );
    assert_eq!(
        sub(zero, zero, zero, p_plus_one),
        Err(ExitCode::MalformedBuiltinParams)
    );
    let p_le: [u8; N] = biguint_to_le(modulus);
    assert_eq!(sub(zero, zero, p_le, p_le).unwrap(), (zero, zero));
}

#[test]
fn fp1_bn254_matches_arkworks() {
    check_fp1::<ark_bn254::Fq, BN254_FP_SIZE>(
        &Bn254BaseField::modulus(),
        syscall_tower_fp1_bn254_add_impl,
        syscall_tower_fp1_bn254_sub_impl,
        syscall_tower_fp1_bn254_mul_impl,
    );
}

#[test]
fn fp1_bls12381_matches_arkworks() {
    check_fp1::<ark_bls12_381::Fq, BLS12381_FP_SIZE>(
        &Bls12381BaseField::modulus(),
        syscall_tower_fp1_bls12381_add_impl,
        syscall_tower_fp1_bls12381_sub_impl,
        syscall_tower_fp1_bls12381_mul_impl,
    );
}

#[test]
fn fp2_bn254_matches_arkworks() {
    check_fp2::<ark_bn254::Fq, ark_bn254::Fq2, BN254_FP_SIZE>(
        &Bn254BaseField::modulus(),
        ark_bn254::Fq2::new,
        |v| (v.c0, v.c1),
        syscall_tower_fp2_bn254_add_impl,
        syscall_tower_fp2_bn254_sub_impl,
        syscall_tower_fp2_bn254_mul_impl,
    );
}

#[test]
fn fp2_bls12381_matches_arkworks() {
    check_fp2::<ark_bls12_381::Fq, ark_bls12_381::Fq2, BLS12381_FP_SIZE>(
        &Bls12381BaseField::modulus(),
        ark_bls12_381::Fq2::new,
        |v| (v.c0, v.c1),
        syscall_tower_fp2_bls12381_add_impl,
        syscall_tower_fp2_bls12381_sub_impl,
        syscall_tower_fp2_bls12381_mul_impl,
    );
}

/// Little-endian `x || y` encoding used by the weierstrass syscalls.
fn ark_point_le<C: AffineRepr, const N: usize>(p: &C) -> [u8; N]
where
    C::BaseField: PrimeField,
{
    let (x, y) = p.xy().expect("non-identity point");
    let mut out = [0u8; N];
    let xb = x.into_bigint().to_bytes_le();
    let yb = y.into_bigint().to_bytes_le();
    out[..xb.len()].copy_from_slice(&xb);
    out[N / 2..N / 2 + yb.len()].copy_from_slice(&yb);
    out
}

fn be_coords_to_le<const N: usize>(x_be: &[u8], y_be: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let (x, y) = out.split_at_mut(N / 2);
    x.copy_from_slice(x_be);
    x.reverse();
    y.copy_from_slice(y_be);
    y.reverse();
    out
}

fn write_le(dst: &mut [u8], v: &BigUint) {
    let bytes = v.to_bytes_le();
    assert!(bytes.len() <= dst.len(), "value does not fit");
    dst.fill(0);
    dst[..bytes.len()].copy_from_slice(&bytes);
}

/// The checks every curve shares: sum and double agree with the reference, equal points are
/// rejected, coordinates at or above the modulus are rejected, and adding `P` to `-P` yields
/// what the SP1 formula yields when the slope denominator is zero (the inverse of zero is zero):
/// `x3 = -2 * px`, `y3 = -py`.
fn check_curve<const N: usize>(
    modulus: &BigUint,
    points: &[([u8; N], [u8; N], [u8; N], [u8; N])], // (p, q, p + q, 2p)
    add: impl Fn([u8; N], [u8; N]) -> Result<[u8; N], ExitCode>,
    double: impl Fn([u8; N]) -> Result<[u8; N], ExitCode>,
) {
    let half = N / 2;
    for (p, q, sum, dbl) in points {
        assert_eq!(add(*p, *q).unwrap(), *sum, "add");
        assert_eq!(add(*q, *p).unwrap(), *sum, "add commutes");
        assert_eq!(double(*p).unwrap(), *dbl, "double");
        assert_eq!(add(*p, *p), Err(ExitCode::MalformedBuiltinParams), "p == q");

        // Coordinates must be canonical.
        let mut bad_x = *p;
        write_le(&mut bad_x[..half], modulus);
        assert_eq!(add(bad_x, *q), Err(ExitCode::MalformedBuiltinParams));
        assert_eq!(add(*q, bad_x), Err(ExitCode::MalformedBuiltinParams));
        assert_eq!(double(bad_x), Err(ExitCode::MalformedBuiltinParams));
        let mut bad_y = *p;
        write_le(&mut bad_y[half..], modulus);
        assert_eq!(add(bad_y, *q), Err(ExitCode::MalformedBuiltinParams));
        assert_eq!(double(bad_y), Err(ExitCode::MalformedBuiltinParams));

        // P + (-P): the SP1 formula with a zero denominator.
        let px = BigUint::from_bytes_le(&p[..half]);
        let py = BigUint::from_bytes_le(&p[half..]);
        let mut neg_p = *p;
        write_le(&mut neg_p[half..], &((modulus - &py) % modulus));
        let mut expected = [0u8; N];
        write_le(
            &mut expected[..half],
            &((modulus * 2u32 - &px - &px) % modulus),
        );
        write_le(&mut expected[half..], &((modulus - &py) % modulus));
        assert_eq!(
            add(*p, neg_p).unwrap(),
            expected,
            "p + (-p) follows the SP1 formula"
        );
    }
}

fn ark_curve_points<C: AffineRepr, const N: usize>() -> Vec<([u8; N], [u8; N], [u8; N], [u8; N])>
where
    C::BaseField: PrimeField,
{
    let mut rng = ark_rng();
    (0..ROUNDS)
        .map(|_| {
            let p = (C::generator() * C::ScalarField::rand(&mut rng)).into_affine();
            let q = (C::generator() * C::ScalarField::rand(&mut rng)).into_affine();
            let sum = (p.into_group() + q.into_group()).into_affine();
            let dbl = (p.into_group() + p.into_group()).into_affine();
            (
                ark_point_le::<C, N>(&p),
                ark_point_le::<C, N>(&q),
                ark_point_le::<C, N>(&sum),
                ark_point_le::<C, N>(&dbl),
            )
        })
        .collect()
}

#[test]
fn bn254_g1_add_and_double_match_arkworks() {
    check_curve::<BN254_G1_RAW_AFFINE_SIZE>(
        &Bn254BaseField::modulus(),
        &ark_curve_points::<ark_bn254::G1Affine, BN254_G1_RAW_AFFINE_SIZE>(),
        syscall_bn254_add_impl,
        syscall_bn254_double_impl,
    );
}

#[test]
fn bls12381_g1_add_and_double_match_arkworks() {
    check_curve::<BLS12381_G1_RAW_AFFINE_SIZE>(
        &Bls12381BaseField::modulus(),
        &ark_curve_points::<ark_bls12_381::G1Affine, BLS12381_G1_RAW_AFFINE_SIZE>(),
        syscall_bls12381_add_impl,
        syscall_bls12381_double_impl,
    );
}

fn k256_point_le(p: &k256::ProjectivePoint) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
    let enc = p.to_affine().to_encoded_point(false);
    be_coords_to_le(enc.x().unwrap(), enc.y().unwrap())
}

fn p256_point_le(p: &p256::ProjectivePoint) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
    let enc = p.to_affine().to_encoded_point(false);
    be_coords_to_le(enc.x().unwrap(), enc.y().unwrap())
}

#[test]
fn secp256k1_add_and_double_match_k256() {
    let mut rng = rng();
    let points: Vec<_> = (0..ROUNDS)
        .map(|_| {
            let p = k256::ProjectivePoint::GENERATOR * k256::Scalar::from(rng.random::<u64>() | 1);
            let q = k256::ProjectivePoint::GENERATOR * k256::Scalar::from(rng.random::<u64>() | 2);
            (
                k256_point_le(&p),
                k256_point_le(&q),
                k256_point_le(&(p + q)),
                k256_point_le(&(p + p)),
            )
        })
        .collect();
    check_curve::<SECP256K1_G1_RAW_AFFINE_SIZE>(
        &Secp256k1BaseField::modulus(),
        &points,
        syscall_secp256k1_add_impl,
        syscall_secp256k1_double_impl,
    );
}

#[test]
fn secp256r1_add_and_double_match_p256() {
    let mut rng = rng();
    let points: Vec<_> = (0..ROUNDS)
        .map(|_| {
            let p = p256::ProjectivePoint::GENERATOR * p256::Scalar::from(rng.random::<u64>() | 1);
            let q = p256::ProjectivePoint::GENERATOR * p256::Scalar::from(rng.random::<u64>() | 2);
            (
                p256_point_le(&p),
                p256_point_le(&q),
                p256_point_le(&(p + q)),
                p256_point_le(&(p + p)),
            )
        })
        .collect();
    check_curve::<SECP256R1_G1_RAW_AFFINE_SIZE>(
        &Secp256r1BaseField::modulus(),
        &points,
        syscall_secp256r1_add_impl,
        syscall_secp256r1_double_impl,
    );
}

/// Decompression output is `y || x` in little-endian; the sign bit selects the odd `y` for the
/// secp curves and the lexicographically larger `y` for BLS12-381 (the ZCash flag convention).
#[test]
fn secp256k1_decompress_round_trips() {
    let mut rng = rng();
    for _ in 0..ROUNDS {
        let p = k256::ProjectivePoint::GENERATOR * k256::Scalar::from(rng.random::<u64>() | 1);
        let enc = p.to_affine().to_encoded_point(false);
        let mut x_le = [0u8; SECP256K1_G1_COMPRESSED_SIZE];
        x_le.copy_from_slice(enc.x().unwrap());
        x_le.reverse();
        let y_be = enc.y().unwrap();
        let sign = u32::from(y_be[31] & 1);
        let out = syscall_secp256k1_decompress_impl(x_le, sign).unwrap();
        let mut y_le = [0u8; 32];
        y_le.copy_from_slice(y_be);
        y_le.reverse();
        assert_eq!(&out[..32], &y_le, "y");
        assert_eq!(&out[32..], &x_le, "x");
    }
    assert_eq!(
        syscall_secp256k1_decompress_impl([0u8; SECP256K1_G1_COMPRESSED_SIZE], 2),
        Err(ExitCode::MalformedBuiltinParams)
    );
    // `x` at the modulus is not a field element.
    let x_le: [u8; SECP256K1_G1_COMPRESSED_SIZE] = biguint_to_le(&Secp256k1BaseField::modulus());
    assert_eq!(
        syscall_secp256k1_decompress_impl(x_le, 0),
        Err(ExitCode::MalformedBuiltinParams)
    );
    let _ = rng.random::<u8>();
}

#[test]
fn secp256r1_decompress_round_trips() {
    let mut rng = rng();
    for _ in 0..ROUNDS {
        let p = p256::ProjectivePoint::GENERATOR * p256::Scalar::from(rng.random::<u64>() | 1);
        let enc = p.to_affine().to_encoded_point(false);
        let mut x_le = [0u8; SECP256R1_G1_COMPRESSED_SIZE];
        x_le.copy_from_slice(enc.x().unwrap());
        x_le.reverse();
        let y_be = enc.y().unwrap();
        let sign = u32::from(y_be[31] & 1);
        let out = syscall_secp256r1_decompress_impl(x_le, sign).unwrap();
        let mut y_le = [0u8; 32];
        y_le.copy_from_slice(y_be);
        y_le.reverse();
        assert_eq!(&out[..32], &y_le, "y");
        assert_eq!(&out[32..], &x_le, "x");
    }
}

#[test]
fn bls12381_decompress_round_trips() {
    let mut rng = ark_rng();
    let modulus = Bls12381BaseField::modulus();
    let half = (&modulus - 1u32) / 2u32;
    for _ in 0..ROUNDS {
        let p = (ark_bls12_381::G1Affine::generator() * ark_bls12_381::Fr::rand(&mut rng))
            .into_affine();
        let (x, y) = p.xy().unwrap();
        let x_le = fe_to_le::<ark_bls12_381::Fq, BLS12381_FP_SIZE>(&x);
        let y_le = fe_to_le::<ark_bls12_381::Fq, BLS12381_FP_SIZE>(&y);
        let sign = u32::from(BigUint::from_bytes_le(&y_le) > half);
        let out = syscall_bls12381_decompress_impl(x_le, sign).unwrap();
        assert_eq!(&out[..BLS12381_FP_SIZE], &y_le, "y");
        assert_eq!(&out[BLS12381_FP_SIZE..], &x_le, "x");
    }
    assert_eq!(
        syscall_bls12381_decompress_impl([0u8; BLS12381_G1_COMPRESSED_SIZE], 2),
        Err(ExitCode::MalformedBuiltinParams)
    );
}

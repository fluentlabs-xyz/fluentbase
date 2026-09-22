//! Fixed-width field and curve arithmetic behind the tower and weierstrass syscalls.
//!
//! The syscall ABI is canonical little-endian bytes in and out. Inside the handler the operands
//! live in arkworks' Montgomery representation; results are identical to the arbitrary-precision
//! integer arithmetic these handlers used before (`(a op b) mod p`), only the representation
//! changed. The raw-integer rules of the original SP1 semantics are kept explicitly: subtraction
//! rejects `b > a + p` on the unreduced inputs, curve operations reject coordinates at or above
//! the modulus, and a zero slope denominator inverts to zero.

use ark_ff::{AdditiveGroup, BigInteger, PrimeField};
use sp1_curves::weierstrass::{
    bls12_381::{Bls12381, Bls12381BaseField},
    bn254::{Bn254, Bn254BaseField},
    secp256k1::{Secp256k1, Secp256k1BaseField},
    secp256r1::{Secp256r1, Secp256r1BaseField},
};

/// Maps an SP1 field parameter type onto the fixed-width field that implements it.
pub(crate) trait NativeField {
    type Fq: PrimeField;
}

impl NativeField for Bn254BaseField {
    type Fq = ark_bn254::Fq;
}
impl NativeField for Bls12381BaseField {
    type Fq = ark_bls12_381::Fq;
}
impl NativeField for Secp256k1BaseField {
    type Fq = ark_secp256k1::Fq;
}
impl NativeField for Secp256r1BaseField {
    type Fq = ark_secp256r1::Fq;
}

/// Maps an SP1 curve type onto its coordinate field and short-Weierstrass `a` coefficient.
pub(crate) trait NativeCurve {
    type Fq: PrimeField;
    fn coeff_a() -> Self::Fq;
}

impl NativeCurve for Bn254 {
    type Fq = ark_bn254::Fq;
    fn coeff_a() -> Self::Fq {
        ark_bn254::Fq::ZERO
    }
}
impl NativeCurve for Bls12381 {
    type Fq = ark_bls12_381::Fq;
    fn coeff_a() -> Self::Fq {
        ark_bls12_381::Fq::ZERO
    }
}
impl NativeCurve for Secp256k1 {
    type Fq = ark_secp256k1::Fq;
    fn coeff_a() -> Self::Fq {
        ark_secp256k1::Fq::ZERO
    }
}
impl NativeCurve for Secp256r1 {
    type Fq = ark_secp256r1::Fq;
    fn coeff_a() -> Self::Fq {
        -ark_secp256r1::Fq::from(3u64)
    }
}

/// Little-endian bytes to a field element, reducing non-canonical inputs modulo `p`.
///
/// Canonical inputs of exactly the limb width take the allocation-free path; anything else goes
/// through arkworks' generic reduction.
#[inline]
pub(crate) fn fe_from_le<F: PrimeField>(bytes: &[u8]) -> F {
    let mut repr = F::BigInt::default();
    let limbs: &mut [u64] = repr.as_mut();
    if bytes.len() == limbs.len() * 8 {
        for (limb, chunk) in limbs.iter_mut().zip(bytes.chunks_exact(8)) {
            *limb = u64::from_le_bytes(chunk.try_into().expect("8-byte chunk"));
        }
        if let Some(value) = F::from_bigint(repr) {
            return value;
        }
    }
    F::from_le_bytes_mod_order(bytes)
}

/// Field element to canonical little-endian bytes, zero-padded to `N`.
#[inline]
pub(crate) fn fe_to_le<F: PrimeField, const N: usize>(value: F) -> [u8; N] {
    let mut out = [0u8; N];
    let repr = value.into_bigint();
    let limbs: &[u64] = repr.as_ref();
    debug_assert!(limbs.len() * 8 <= N);
    for (chunk, limb) in out.chunks_exact_mut(8).zip(limbs) {
        chunk.copy_from_slice(&limb.to_le_bytes());
    }
    out
}

/// Affine point to the syscall layout `x || y`, each coordinate `N / 2` little-endian bytes.
#[inline]
pub(crate) fn encode_point<F: PrimeField, const N: usize>(x: F, y: F) -> [u8; N] {
    let mut out = [0u8; N];
    let (x_dst, y_dst) = out.split_at_mut(N / 2);
    for (dst, value) in [(x_dst, x), (y_dst, y)] {
        let repr = value.into_bigint();
        let limbs: &[u64] = repr.as_ref();
        debug_assert!(limbs.len() * 8 <= dst.len());
        for (chunk, limb) in dst.chunks_exact_mut(8).zip(limbs) {
            chunk.copy_from_slice(&limb.to_le_bytes());
        }
    }
    out
}

/// Canonical little-endian bytes of the modulus, zero-padded to `N`.
#[inline]
pub(crate) fn modulus_le<F: PrimeField, const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = F::MODULUS.to_bytes_le();
    debug_assert!(bytes.len() <= N);
    out[..bytes.len()].copy_from_slice(&bytes);
    out
}

/// `a >= b` on little-endian byte strings of equal length.
#[inline]
pub(crate) fn le_ge(a: &[u8], b: &[u8]) -> bool {
    debug_assert_eq!(a.len(), b.len());
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

/// `a + m < b` on little-endian byte strings of equal length, computed on the raw integers.
///
/// This is the underflow rule the tower subtraction syscalls apply before reducing.
#[inline]
pub(crate) fn le_add_lt(a: &[u8], m: &[u8], b: &[u8]) -> bool {
    debug_assert!(a.len() == m.len() && m.len() == b.len());
    // Compute `a + m` little-endian with a carry-out; the sum has one more byte than `b`.
    let mut sum = [0u8; 65];
    let mut carry = 0u16;
    for i in 0..a.len() {
        let s = u16::from(a[i]) + u16::from(m[i]) + carry;
        sum[i] = s as u8;
        carry = s >> 8;
    }
    if carry != 0 {
        return false;
    }
    !le_ge(&sum[..a.len()], b)
}

use crate::{
    utils::{AffinePoint, WeierstrassAffinePoint, WeierstrassPoint},
    MathRuntime,
};
use fluentbase_sdk::MathAPI;
use std::io::ErrorKind;

/// The number of limbs in [Bls12381AffinePoint].
pub const N: usize = 24;

/// A point on the BLS12-381 curve.
#[derive(Copy, Clone)]
#[repr(align(4))]
pub struct Bls12381Point(pub WeierstrassPoint<N>);

impl WeierstrassAffinePoint<N> for Bls12381Point {
    fn infinity() -> Self {
        Self(WeierstrassPoint::Infinity)
    }

    fn is_infinity(&self) -> bool {
        matches!(self.0, WeierstrassPoint::Infinity)
    }
}

impl AffinePoint<N> for Bls12381Point {
    const GENERATOR: [u32; N] = [
        3676489403, 4214943754, 4185529071, 1817569343, 387689560, 2706258495, 2541009157,
        3278408783, 1336519695, 647324556, 832034708, 401724327, 1187375073, 212476713, 2726857444,
        3493644100, 738505709, 14358731, 3587181302, 4243972245, 1948093156, 2694721773,
        3819610353, 146011265,
    ];

    /// The generator was taken from "py_ecc" python library by the Ethereum Foundation:
    ///
    /// https://github.com/ethereum/py_ecc/blob/7b9e1b3/py_ecc/bls12_381/bls12_381_curve.py#L38-L45
    #[allow(deprecated)]
    const GENERATOR_T: Self = Self(WeierstrassPoint::Affine(Self::GENERATOR));

    fn new(limbs: [u32; N]) -> Self {
        Self(WeierstrassPoint::Affine(limbs))
    }

    fn identity() -> Self {
        Self::infinity()
    }

    fn is_identity(&self) -> bool {
        self.is_infinity()
    }

    fn limbs_ref(&self) -> &[u32; N] {
        match &self.0 {
            WeierstrassPoint::Infinity => panic!("Infinity point has no limbs"),
            WeierstrassPoint::Affine(limbs) => limbs,
        }
    }

    fn limbs_mut(&mut self) -> &mut [u32; N] {
        match &mut self.0 {
            WeierstrassPoint::Infinity => panic!("Infinity point has no limbs"),
            WeierstrassPoint::Affine(limbs) => limbs,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        let a = bytemuck::cast_mut(self.limbs_mut());
        let b = bytemuck::cast_ref(other.limbs_ref());
        MathRuntime::bls12_381_g1_add(a, b);
    }

    fn complete_add_assign(&mut self, other: &Self) {
        self.weierstrass_add_assign(other);
    }

    fn double(&mut self) {
        let _a = self.limbs_mut();
        unimplemented!("where is bls12_381_double?")
    }
}

/// Decompresses a compressed public key using bls12381_decompress precompile.
pub fn decompress_pubkey(compressed_key: &[u8; 48]) -> Result<[u8; 96], ErrorKind> {
    let mut decompressed_key = [0u8; 96];
    decompressed_key[..48].copy_from_slice(compressed_key);

    let sign_bit = ((decompressed_key[0] & 0b_0010_0000) >> 5) == 1;
    decompressed_key[0] &= 0b_0001_1111;
    unimplemented!("where is bls12_381_decompress?");

    Ok(decompressed_key)
}

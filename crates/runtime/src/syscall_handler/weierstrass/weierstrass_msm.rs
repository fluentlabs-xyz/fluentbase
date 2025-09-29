//! Handles the syscall for multi-scalar multiplication on a Weierstrass curve.
//! Expects parameters: (pairs_ptr, pairs_len, out_ptr)

use super::config::MulConfig;
use crate::syscall_handler::weierstrass::{
    g2_be_uncompressed_to_le_limbs, g2_le_limbs_to_be_uncompressed, helpers_bls::parse_affine_g2,
    parse_bls12381_g1_point_uncompressed,
};
use crate::RuntimeContext;
use blstrs::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use fluentbase_types::{G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE, SCALAR_SIZE};
use group::Group;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

pub struct SyscallWeierstrassMsm<C: MulConfig> {
    _phantom: PhantomData<C>,
}

impl<C: MulConfig> SyscallWeierstrassMsm<C> {
    /// Create a new instance of the [`SyscallWeierstrassMsm`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let pairs_ptr = params[0].i32().unwrap() as usize;
        let pairs_len = params[1].i32().unwrap() as usize;
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair: point + scalar
        let pair_size = C::POINT_SIZE + C::SCALAR_SIZE;
        let total_len = pairs_len * pair_size;
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // Parse into pairs of (point, scalar)
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * pair_size;
            let mut point = vec![0u8; C::POINT_SIZE];
            let mut scalar = vec![0u8; C::SCALAR_SIZE];
            point.copy_from_slice(&buf[start..start + C::POINT_SIZE]);
            scalar.copy_from_slice(&buf[start + C::POINT_SIZE..start + pair_size]);
            pairs.push((point, scalar));
        }

        let result = Self::fn_impl(&pairs);
        if !result.is_empty() {
            caller.memory_write(out_ptr, &result)?;
        }

        Ok(())
    }

    pub fn fn_impl(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        match C::CURVE_TYPE {
            CurveType::Bls12381 => match C::POINT_SIZE {
                G1_UNCOMPRESSED_SIZE => Self::fn_impl_bls12381_g1(pairs),
                G2_UNCOMPRESSED_SIZE => Self::fn_impl_bls12381_g2(pairs),
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// BLS12-381 G1 multi-scalar multiplication implementation
    fn fn_impl_bls12381_g1(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut acc = G1Projective::identity();

        for (point_bytes, scalar_bytes) in pairs.iter() {
            let mut point = [0u8; G1_UNCOMPRESSED_SIZE];
            let mut scalar = [0u8; SCALAR_SIZE];

            point.copy_from_slice(&point_bytes[..G1_UNCOMPRESSED_SIZE.min(point_bytes.len())]);
            scalar.copy_from_slice(&scalar_bytes[..SCALAR_SIZE.min(scalar_bytes.len())]);

            let point_aff = parse_bls12381_g1_point_uncompressed(&point);

            // Skip if scalar is zero
            if scalar.iter().all(|&b| b == 0) {
                continue;
            }

            let scalar_scalar = Scalar::from_bytes_be(&scalar);
            if scalar_scalar.is_none().unwrap_u8() == 1 {
                continue;
            }

            let scalar_val = scalar_scalar.unwrap();
            acc += &(G1Projective::from(point_aff) * &scalar_val);
        }

        // If identity, return zeroed result
        if acc.is_identity().unwrap_u8() == 1 {
            return vec![0u8; G1_UNCOMPRESSED_SIZE];
        }

        // Serialize result
        G1Affine::from(acc).to_uncompressed().to_vec()
    }

    /// BLS12-381 G2 multi-scalar multiplication implementation
    fn fn_impl_bls12381_g2(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut acc = G2Projective::identity();

        for (point_bytes, scalar_bytes) in pairs.iter() {
            let mut point = [0u8; G2_UNCOMPRESSED_SIZE];
            let mut scalar = [0u8; SCALAR_SIZE];

            point.copy_from_slice(&point_bytes[..G2_UNCOMPRESSED_SIZE.min(point_bytes.len())]);
            scalar.copy_from_slice(&scalar_bytes[..SCALAR_SIZE.min(scalar_bytes.len())]);

            // Convert point to BE uncompressed using shared helper
            let be = g2_le_limbs_to_be_uncompressed(&point);

            let a_aff = parse_affine_g2(&be);
            let a = G2Projective::from(a_aff);
            let scalar_opt = Scalar::from_bytes_be(&scalar);
            if scalar_opt.is_none().unwrap_u8() == 1 {
                continue;
            }
            let scalar_val = scalar_opt.unwrap();
            acc += &(a * &scalar_val);
        }

        // If identity, return zeroed limbs (runtime convention)
        if acc.is_identity().unwrap_u8() == 1 {
            return vec![0u8; G2_UNCOMPRESSED_SIZE];
        }

        // Serialize acc to BE uncompressed and convert back to LE limbs
        let sum_aff = G2Affine::from(acc);
        g2_be_uncompressed_to_le_limbs(&sum_aff.to_uncompressed()).to_vec()
    }
}

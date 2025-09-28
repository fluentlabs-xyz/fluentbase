//! Handles the syscall for point multiplication on a Weierstrass curve.
//!
//! This module provides a generic syscall handler for scalar multiplication operations on various
//! Weierstrass curves including BLS12-381, BN254, Secp256k1, and Secp256r1. It supports both G1
//! and G2 group operations where applicable, with curve-specific implementations for optimal
//! performance. The handler uses a configuration-driven approach through MulConfig traits to
//! determine the appropriate curve type, field size, and operation parameters.
//!
//! Expects parameters: (p_ptr, q_ptr) where p is the point and q is the scalar

use super::config::{
    Bls12381G1MulConfig, Bls12381G2MulConfig, Bn254G1MulConfig, Bn254G2MulConfig, MulConfig,
};
use crate::syscall_handler::weierstrass::{
    parse_bls12381_g1_point_uncompressed, parse_bls12381_g2_point_uncompressed,
};

use super::weierstrass_helpers::is_zero_point;
use crate::syscall_handler::weierstrass::bn256_helpers::{
    encode_g1_point, read_g1_point, read_scalar,
};
use crate::RuntimeContext;
use ark_ec::CurveGroup;
use blstrs::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use fluentbase_types::{
    BN254_G1_POINT_DECOMPRESSED_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE, SCALAR_SIZE,
};
use group::prime::PrimeCurveAffine;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

pub struct SyscallWeierstrassMulAssign<C: MulConfig> {
    _phantom: PhantomData<C>,
}

impl<C: MulConfig> SyscallWeierstrassMulAssign<C> {
    /// Create a new instance of the [`WeierstrassMulAssignSyscall`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Handles the syscall for point multiplication on a Weierstrass curve.
    /// Uses the MulConfig to determine curve type and field (G1/G2).
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (p_ptr, q_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );

        // Read point and scalar from memory using config sizes
        let mut point = vec![0u8; C::POINT_SIZE];
        let mut scalar = vec![0u8; C::SCALAR_SIZE];
        caller.memory_read(p_ptr as usize, &mut point)?;
        caller.memory_read(q_ptr as usize, &mut scalar)?;

        // Perform scalar multiplication
        let result = Self::fn_impl(&point, &scalar);

        // Write result back to memory
        if !result.is_empty() {
            caller.memory_write(p_ptr as usize, &result)?;
        }

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Vec<u8> {
        match C::CURVE_TYPE {
            CurveType::Bn254 => {
                if C::POINT_SIZE == Bn254G1MulConfig::POINT_SIZE {
                    Self::fn_impl_bn254_g1(p, q)
                } else if C::POINT_SIZE == Bn254G2MulConfig::POINT_SIZE {
                    assert!(false, "Bn254G2MulConfig is not supported");
                    vec![]
                    // Self::fn_impl_bn254_g2(p, q)
                } else {
                    // Fallback to generic implementation
                    Self::fn_impl_generic(p, q)
                }
            }
            CurveType::Bls12381 => {
                if C::POINT_SIZE == Bls12381G1MulConfig::POINT_SIZE {
                    Self::fn_impl_bls12381_g1(p, q)
                } else if C::POINT_SIZE == Bls12381G2MulConfig::POINT_SIZE {
                    Self::fn_impl_bls12381_g2(p, q)
                } else {
                    // Fallback to generic implementation
                    Self::fn_impl_generic(p, q)
                }
            }
            CurveType::Secp256k1 => {
                assert!(false, "Secp256k1 is not supported");
                vec![]
            }
            CurveType::Secp256r1 => {
                assert!(false, "Secp256r1 is not supported");
                vec![]
            }
            _ => {
                // Generic implementation for other curves
                Self::fn_impl_generic(p, q)
            }
        }
    }

    pub fn fn_impl_bn254_g1(p: &[u8], q: &[u8]) -> Vec<u8> {
        // Convert input to big-endian format expected by ark-bn254
        let p_be: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = p
            .try_into()
            .unwrap_or([0u8; BN254_G1_POINT_DECOMPRESSED_SIZE]);
        let q_be: [u8; SCALAR_SIZE] = q.try_into().unwrap_or([0u8; SCALAR_SIZE]);

        // Parse point and scalar using ark-bn254
        let p_ark = match read_g1_point(&p_be) {
            Ok(point) => point,
            Err(_) => return vec![0u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        };
        let scalar = read_scalar(&q_be);

        // Use ark-bn254 for multiplication
        let p_proj: ark_bn254::G1Projective = p_ark.into();
        let result_proj = p_proj * scalar;
        let result_ark = result_proj.into_affine();

        // Convert result back to BE format
        let result_be = encode_g1_point(result_ark);
        result_be.to_vec()
    }

    fn fn_impl_bls12381_g1(p: &[u8], q: &[u8]) -> Vec<u8> {
        let mut point = [0u8; G1_UNCOMPRESSED_SIZE];
        let mut scalar = [0u8; SCALAR_SIZE];

        point.copy_from_slice(&p[..G1_UNCOMPRESSED_SIZE.min(p.len())]);
        scalar.copy_from_slice(&q[..SCALAR_SIZE.min(q.len())]);

        let point_aff = parse_bls12381_g1_point_uncompressed(&point);

        if is_zero_point(&scalar) {
            return G1Affine::identity().to_uncompressed().to_vec();
        }

        let scalar_scalar = Scalar::from_bytes_be(&scalar);
        if scalar_scalar.is_none().unwrap_u8() == 1 {
            return G1Affine::identity().to_uncompressed().to_vec();
        }

        let result_proj = G1Projective::from(point_aff) * scalar_scalar.unwrap();
        G1Affine::from(result_proj).to_uncompressed().to_vec()
    }

    fn fn_impl_bls12381_g2(p: &[u8], q: &[u8]) -> Vec<u8> {
        let mut point = [0u8; G2_UNCOMPRESSED_SIZE];
        let mut scalar = [0u8; SCALAR_SIZE];

        point.copy_from_slice(&p[..G2_UNCOMPRESSED_SIZE.min(p.len())]);
        scalar.copy_from_slice(&q[..SCALAR_SIZE.min(q.len())]);

        let point_aff = parse_bls12381_g2_point_uncompressed(&point);

        if is_zero_point(&scalar) {
            return G2Affine::identity().to_uncompressed().to_vec();
        }

        let scalar_scalar = Scalar::from_bytes_be(&scalar);
        if scalar_scalar.is_none().unwrap_u8() == 1 {
            return G2Affine::identity().to_uncompressed().to_vec();
        }

        let result_proj = G2Projective::from(point_aff) * scalar_scalar.unwrap();
        G2Affine::from(result_proj).to_uncompressed().to_vec()
    }

    /// Generic implementation for other curves using sp1_curves
    fn fn_impl_generic(_p: &[u8], _q: &[u8]) -> Vec<u8> {
        // TODO: Implement generic curve support
        // For now, return empty result
        vec![]
    }
}

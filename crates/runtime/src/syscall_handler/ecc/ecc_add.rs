use super::ecc_config::AddConfig;
use crate::{
    syscall_handler::{
        cast_u8_to_u32,
        ecc::{
            ecc_bls12381::parse_affine_g2, ecc_bn256, g2_be_uncompressed_to_le_limbs,
            g2_le_limbs_to_be_uncompressed, parse_bls12381_g1_point_uncompressed,
        },
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use ark_ec::CurveGroup;
use blstrs::{G1Affine, G1Projective, G2Affine, G2Projective};
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE,
};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{AffinePoint, CurveType, EllipticCurve};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallEccAdd<C: AddConfig> {
    _phantom: PhantomData<C>,
}

impl<C: AddConfig> SyscallEccAdd<C> {
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
        let (p_ptr, q_ptr) = (
            params[0].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[1].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
        );

        let mut p = vec![0u8; C::POINT_SIZE];
        caller.memory_read(p_ptr as usize, &mut p)?;
        let mut q = vec![0u8; C::POINT_SIZE];
        caller.memory_read(q_ptr as usize, &mut q)?;

        let result_vec = Self::fn_impl(&p, &q).map_err(|e| syscall_process_exit_code(caller, e))?;
        if !result_vec.is_empty() {
            caller.memory_write(p_ptr as usize, &result_vec)?;
        }

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Result<Vec<u8>, ExitCode> {
        match C::CURVE_TYPE {
            CurveType::Bls12381 => {
                if C::POINT_SIZE == G2_UNCOMPRESSED_SIZE {
                    let p_array: [u8; G2_UNCOMPRESSED_SIZE] =
                        p.try_into().unwrap_or([0u8; G2_UNCOMPRESSED_SIZE]);
                    let q_array: [u8; G2_UNCOMPRESSED_SIZE] =
                        q.try_into().unwrap_or([0u8; G2_UNCOMPRESSED_SIZE]);
                    Ok(Self::fn_impl_bls12381_g2(&p_array, &q_array))
                } else {
                    let p_array: [u8; G1_UNCOMPRESSED_SIZE] =
                        p.try_into().unwrap_or([0u8; G1_UNCOMPRESSED_SIZE]);
                    let q_array: [u8; G1_UNCOMPRESSED_SIZE] =
                        q.try_into().unwrap_or([0u8; G1_UNCOMPRESSED_SIZE]);
                    Ok(Self::fn_impl_bls12381_g1(&p_array, &q_array))
                }
            }
            CurveType::Bn254 => Self::fn_impl_bn254(p, q),
            _ => {
                let p_words = match cast_u8_to_u32(p) {
                    Some(words) => words,
                    None => return Ok(vec![]),
                };
                let q_words = match cast_u8_to_u32(q) {
                    Some(words) => words,
                    None => return Ok(vec![]),
                };

                let p_aff = AffinePoint::<C::EllipticCurve>::from_words_le(&p_words);
                let q_aff = AffinePoint::<C::EllipticCurve>::from_words_le(&q_words);

                let r_aff = C::EllipticCurve::ec_add(&p_aff, &q_aff);

                let r_words = r_aff.to_words_le();
                Ok(words_to_bytes_le_vec(r_words.as_slice()))
            }
        }
    }

    fn fn_impl_bn254(p: &[u8], q: &[u8]) -> Result<Vec<u8>, ExitCode> {
        // Convert input to fixed-size arrays
        let p_array: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = match p.try_into() {
            Ok(arr) => arr,
            Err(_) => return Err(ExitCode::MalformedBuiltinParams),
        };
        let q_array: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = match q.try_into() {
            Ok(arr) => arr,
            Err(_) => return Err(ExitCode::MalformedBuiltinParams),
        };

        let p1 = ecc_bn256::read_g1_point(&p_array)?;
        let p2 = ecc_bn256::read_g1_point(&q_array)?;

        let p1_jacobian: ark_bn254::G1Projective = p1.into();
        let p3 = p1_jacobian + p2;

        let output = ecc_bn256::encode_g1_point(p3.into_affine());
        Ok(output.to_vec())
    }

    /// BLS12-381 G1 point addition implementation
    fn fn_impl_bls12381_g1(
        p: &[u8; G1_UNCOMPRESSED_SIZE],
        q: &[u8; G1_UNCOMPRESSED_SIZE],
    ) -> Vec<u8> {
        let p_aff = parse_bls12381_g1_point_uncompressed(p);
        let q_aff = parse_bls12381_g1_point_uncompressed(q);

        let result_proj = G1Projective::from(p_aff) + G1Projective::from(q_aff);
        let result_aff = G1Affine::from(result_proj);
        result_aff.to_uncompressed().to_vec()
    }

    fn fn_impl_bls12381_g2(
        p: &[u8; G2_UNCOMPRESSED_SIZE],
        q: &[u8; G2_UNCOMPRESSED_SIZE],
    ) -> Vec<u8> {
        // p, q layout: x0||x1||y0||y1, each limb 48 bytes little-endian
        // Convert to blstrs uncompressed big-endian bytes with c0/c1 swapped, add, then convert back.

        let a_be = g2_le_limbs_to_be_uncompressed(p);
        let b_be = g2_le_limbs_to_be_uncompressed(q);

        let a_aff = parse_affine_g2(&a_be);
        let b_aff = parse_affine_g2(&b_be);

        let sum = G2Projective::from(a_aff) + G2Projective::from(b_aff);
        let sum_aff = G2Affine::from(sum);

        let be_result = sum_aff.to_uncompressed();
        let le_result = g2_be_uncompressed_to_le_limbs(&be_result);

        le_result.to_vec()
    }
}

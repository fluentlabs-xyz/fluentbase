use super::ecc_config::{
    Bls12381G1MulConfig, Bls12381G2MulConfig, Bn254G1MulConfig, Bn254G2MulConfig, MulConfig,
};
use crate::{
    syscall_handler::{
        ecc::{
            ecc_bn256::{encode_g1_point, is_zero_point, read_g1_point, read_scalar},
            parse_bls12381_g1_point_uncompressed, parse_bls12381_g2_point_uncompressed,
        },
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use ark_ec::CurveGroup;
use blstrs::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use fluentbase_sdk::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE,
    SCALAR_SIZE,
};
use group::prime::PrimeCurveAffine;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

pub struct SyscallEccMul<C: MulConfig> {
    _phantom: PhantomData<C>,
}

impl<C: MulConfig> SyscallEccMul<C> {
    /// Create a new instance of the [`WeierstrassMulAssignSyscall`].
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

        // Read point and scalar from memory using config sizes
        let mut point = vec![0u8; C::POINT_SIZE];
        let mut scalar = vec![0u8; C::SCALAR_SIZE];
        caller.memory_read(p_ptr as usize, &mut point)?;
        caller.memory_read(q_ptr as usize, &mut scalar)?;

        // Perform scalar multiplication with proper error handling
        let result =
            Self::fn_impl(&point, &scalar).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(result_data) = result {
            caller.memory_write(p_ptr as usize, &result_data)?;
        }

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Result<Vec<u8>, ExitCode> {
        match C::CURVE_TYPE {
            CurveType::Bn254 => {
                if C::POINT_SIZE == Bn254G1MulConfig::POINT_SIZE {
                    Self::fn_impl_bn254_g1(p, q)
                } else if C::POINT_SIZE == Bn254G2MulConfig::POINT_SIZE {
                    // Bn254G2MulConfig is not currently supported
                    Err(ExitCode::MalformedBuiltinParams)
                } else {
                    Err(ExitCode::MalformedBuiltinParams)
                }
            }
            CurveType::Bls12381 => {
                if C::POINT_SIZE == Bls12381G1MulConfig::POINT_SIZE {
                    Ok(Self::fn_impl_bls12381_g1(p, q))
                } else if C::POINT_SIZE == Bls12381G2MulConfig::POINT_SIZE {
                    Ok(Self::fn_impl_bls12381_g2(p, q))
                } else {
                    Err(ExitCode::MalformedBuiltinParams)
                }
            }
            CurveType::Secp256k1 => {
                // Secp256k1 scalar multiplication is not currently supported
                Err(ExitCode::MalformedBuiltinParams)
            }
            CurveType::Secp256r1 => {
                // Secp256r1 scalar multiplication is not currently supported
                Err(ExitCode::MalformedBuiltinParams)
            }
            _ => Err(ExitCode::MalformedBuiltinParams),
        }
    }

    pub fn fn_impl_bn254_g1(p: &[u8], q: &[u8]) -> Result<Vec<u8>, ExitCode> {
        // Convert input to big-endian format expected by ark-bn254
        let p_be: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] =
            p.try_into().map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let q_be: [u8; SCALAR_SIZE] = q.try_into().map_err(|_| ExitCode::MalformedBuiltinParams)?;

        // Parse point and scalar using ark-bn254
        let p_ark = read_g1_point(&p_be).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let scalar = read_scalar(&q_be);

        // Use ark-bn254 for multiplication
        let p_proj: ark_bn254::G1Projective = p_ark.into();
        let result_proj = p_proj * scalar;
        let result_ark = result_proj.into_affine();

        // Convert result back to BE format
        let result_be = encode_g1_point(result_ark);
        Ok(result_be.to_vec())
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

        let result_proj =
            G1Projective::from(point_aff) * scalar_scalar.unwrap_or(Scalar::from(0u64));
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

        let result_proj =
            G2Projective::from(point_aff) * scalar_scalar.unwrap_or(Scalar::from(0u64));
        G2Affine::from(result_proj).to_uncompressed().to_vec()
    }
}

use super::bn256_helpers::{encode_g1_point, read_g1_point, read_scalar};
use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{Fr, G1Affine, G1Projective};
use ark_ec::CurveGroup;
use fluentbase_types::{ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, SCALAR_SIZE};
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBn256Mul;

/// Performs point multiplication on a G1 point.
#[inline]
fn bn256_point_mul(p: G1Affine, scalar: Fr) -> G1Affine {
    let p_jacobian: G1Projective = p.into();
    let result = p_jacobian * scalar;
    result.into_affine()
}

impl SyscallBn256Mul {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; SCALAR_SIZE];
        caller.memory_read(q_ptr, &mut q)?;

        let res = Self::fn_impl(&mut p, &q).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(_) = res {
            caller.memory_write(p_ptr, &p)?;
        }

        Ok(())
    }

    pub fn fn_impl(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; SCALAR_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        // Direct implementation matching revm precompile exactly
        let p1 = read_g1_point(p).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let scalar = read_scalar(q);
        let result = bn256_point_mul(p1, scalar);

        let output = encode_g1_point(result);
        p.copy_from_slice(&output);
        Ok(output)
    }
}

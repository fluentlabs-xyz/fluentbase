use super::bn256_helpers::{encode_g1_point, read_g1_point};
use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{G1Affine, G1Projective};
use ark_ec::CurveGroup;
use fluentbase_types::{ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE};
use rwasm::{Store, TrapCode, Value};

pub struct SyscallBn256Add;

/// Performs point addition on two G1 points.
#[inline]
fn g1_point_add(p1: G1Affine, p2: G1Affine) -> G1Affine {
    let p1_jacobian: G1Projective = p1.into();
    let p3 = p1_jacobian + p2;
    p3.into_affine()
}

impl SyscallBn256Add {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
        caller.memory_read(q_ptr, &mut q)?;

        let res = Self::fn_impl(&mut p, &q).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(_) = res {
            caller.memory_write(p_ptr, &p)?;
        }

        Ok(())
    }

    pub fn fn_impl(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        // Direct implementation matching revm precompile exactly
        let p1 = read_g1_point(p).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let p2 = read_g1_point(q).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let result = g1_point_add(p1, p2);

        let output = encode_g1_point(result);
        p.copy_from_slice(&output);
        Ok(output)
    }
}

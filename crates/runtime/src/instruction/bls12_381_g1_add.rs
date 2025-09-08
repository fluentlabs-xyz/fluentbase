use crate::{instruction::weierstrass_add::SyscallWeierstrassAddAssign, RuntimeContext};
use blstrs::{G1Affine, G1Projective};
use group::prime::PrimeCurveAffine;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::{weierstrass::bls12_381::Bls12381, EllipticCurve};

pub struct SyscallBls12381G1Add;

impl SyscallBls12381G1Add {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 96];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; 96];
        caller.memory_read(q_ptr, &mut q)?;

        Self::fn_impl(&mut p, &q);
        caller.memory_write(p_ptr, &p)?;

        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; 96], q: &[u8; 96]) {
        #[inline]
        fn parse_affine(input: &[u8; 96]) -> G1Affine {
            if input.iter().all(|&b| b == 0) {
                // Treat all-zero 96B as identity (used by our ABI for infinity)
                G1Affine::identity()
            } else {
                let ct = G1Affine::from_uncompressed(input);
                if ct.is_none().unwrap_u8() == 1 {
                    // Invalid point encoding
                    // In this syscall context we don't have a Result; use identity to avoid panic
                    // and let higher layers enforce validity as needed.
                    G1Affine::identity()
                } else {
                    ct.unwrap()
                }
            }
        }

        let p_aff = parse_affine(p);
        let q_aff = parse_affine(q);

        let p_proj = G1Projective::from(p_aff);
        let q_proj = G1Projective::from(q_aff);
        let result = p_proj + q_proj;
        let result_aff = G1Affine::from(result);
        let result = result_aff.to_uncompressed();
        p.copy_from_slice(&result);
    }
}

use crate::instruction::bls12_381_helpers::parse_affine_g1;
use crate::RuntimeContext;
use blstrs::{G1Affine, G1Projective};
use rwasm::{Store, TrapCode, TypedCaller, Value};

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
        let p_aff = parse_affine_g1(p);
        let q_aff = parse_affine_g1(q);

        let p_proj = G1Projective::from(p_aff);
        let q_proj = G1Projective::from(q_aff);
        let result = p_proj + q_proj;
        let result_aff = G1Affine::from(result);
        let result = result_aff.to_uncompressed();
        p.copy_from_slice(&result);
    }
}

use crate::{instruction::weierstrass_add::SyscallWeierstrassAddAssign, RuntimeContext};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::bls12_381::Bls12381;

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
        // Both p and q are 96-byte little-endian limbs: x48||y48
        let result = SyscallWeierstrassAddAssign::<Bls12381>::fn_impl(p, q);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }
}

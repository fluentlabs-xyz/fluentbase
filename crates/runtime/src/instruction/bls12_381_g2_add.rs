use crate::{instruction::weierstrass_add::SyscallWeierstrassAddAssign, RuntimeContext};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::bls12_381::Bls12381;

pub struct SyscallBls12381G2Add;

impl SyscallBls12381G2Add {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 64];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; 64];
        caller.memory_read(q_ptr, &mut q)?;

        Self::fn_impl(&mut p, &q);
        caller.memory_write(p_ptr, &p)?;
        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; 64], q: &[u8; 64]) {
        // Expand to 96-byte little-endian (x48||y48)
        let mut p96 = [0u8; 96];
        let mut q96 = [0u8; 96];
        p96[0..32].copy_from_slice(&p[0..32]);
        p96[48..80].copy_from_slice(&p[32..64]);
        q96[0..32].copy_from_slice(&q[0..32]);
        q96[48..80].copy_from_slice(&q[32..64]);

        let result = SyscallWeierstrassAddAssign::<Bls12381>::fn_impl(&p96, &q96);
        // Compress back to 64 bytes
        p[0..32].copy_from_slice(&result[0..32]);
        p[32..64].copy_from_slice(&result[48..80]);
    }
}

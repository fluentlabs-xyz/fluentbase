use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBls12381MapFp2ToG2;

impl SyscallBls12381MapFp2ToG2 {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let out_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 64];
        caller.memory_read(p_ptr, &mut p)?;

        let mut out = [0u8; 64];
        Self::fn_impl(&p, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(p: &[u8; 64], out: &mut [u8; 64]) {
        // Placeholder mapping: copy input to output until proper map is wired.
        out.copy_from_slice(p);
    }
}

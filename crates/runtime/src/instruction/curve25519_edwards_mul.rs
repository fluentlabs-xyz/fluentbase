use crate::{
    instruction::{
        curve25519_edwards_decompress_validate::SyscallCurve25519EdwardsDecompressValidate,
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use curve25519_dalek::EdwardsPoint;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub(crate) struct SyscallCurve25519EdwardsMul {}

impl SyscallCurve25519EdwardsMul {
    pub const fn new() -> Self {
        Self {}
    }
}

impl SyscallCurve25519EdwardsMul {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as u32;
        let q_ptr = params[1].i32().unwrap() as u32;

        let mut p = vec![0; 32];
        caller.memory_read(p_ptr as usize, &mut p)?;

        let mut q = vec![0; 32];
        caller.memory_read(q_ptr as usize, &mut q)?;

        let res = Self::fn_impl(&p.try_into().unwrap(), &q.try_into().unwrap())
            .map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(res) = res {
            caller.memory_write(p_ptr as usize, res.compress().as_bytes())?;
        }
        result[0] = Value::I32(res.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(p: &[u8; 32], q: &[u8; 32]) -> Result<EdwardsPoint, ExitCode> {
        let p = SyscallCurve25519EdwardsDecompressValidate::fn_impl(p)?;
        let q = curve25519_dalek::scalar::Scalar::from_bytes_mod_order(*q);

        let result = p * q;

        Ok(result)
    }
}

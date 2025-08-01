use crate::{
    instruction::{
        ed25519_ristretto_decompress_validate::SyscallED25519RistrettoDecompressValidate,
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use curve25519_dalek::RistrettoPoint;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub(crate) struct SyscallED25519RistrettoSub {}

impl SyscallED25519RistrettoSub {
    pub const fn new() -> Self {
        Self {}
    }
}

impl SyscallED25519RistrettoSub {
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

    pub fn fn_impl(p: &[u8; 32], q: &[u8; 32]) -> Result<RistrettoPoint, ExitCode> {
        let p = SyscallED25519RistrettoDecompressValidate::fn_impl(p)?;
        let q = SyscallED25519RistrettoDecompressValidate::fn_impl(q)?;

        let result = p - q;

        Ok(result)
    }
}

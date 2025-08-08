use crate::{utils::syscall_process_exit_code, RuntimeContext};
use curve25519_dalek::{edwards::CompressedEdwardsY, EdwardsPoint};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub(crate) struct SyscallCurve25519EdwardsDecompressValidate {}

impl SyscallCurve25519EdwardsDecompressValidate {
    pub const fn new() -> Self {
        Self {}
    }
}

impl SyscallCurve25519EdwardsDecompressValidate {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as u32;

        let mut p = vec![0; 32];
        caller.memory_read(p_ptr as usize, &mut p)?;

        let res =
            Self::fn_impl(&p.try_into().unwrap()).map_err(|e| syscall_process_exit_code(caller, e));
        result[0] = Value::I32(res.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(p: &[u8; 32]) -> Result<EdwardsPoint, ExitCode> {
        let compressed = CompressedEdwardsY(p.clone());
        let pt = compressed
            .decompress()
            .ok_or_else(|| ExitCode::MalformedBuiltinParams)?;
        if !pt.is_torsion_free() {
            return Err(ExitCode::MalformedBuiltinParams);
        }
        Ok(pt)
    }
}

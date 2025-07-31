use crate::{instruction::syscall_process_exit_code, RuntimeContext};
use curve25519_dalek::{ristretto::CompressedRistretto, EdwardsPoint, RistrettoPoint};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::EllipticCurve;
use std::marker::PhantomData;

pub(crate) struct SyscallED25519RistrettoDecompressValidate<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallED25519RistrettoDecompressValidate<E> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<E: EllipticCurve> SyscallED25519RistrettoDecompressValidate<E> {
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

    pub fn fn_impl(p: &[u8; 32]) -> Result<RistrettoPoint, ExitCode> {
        let compressed = CompressedRistretto(p.clone());
        let pt = compressed
            .decompress()
            .ok_or_else(|| ExitCode::MalformedBuiltinParams)?;
        Ok(pt)
    }
}

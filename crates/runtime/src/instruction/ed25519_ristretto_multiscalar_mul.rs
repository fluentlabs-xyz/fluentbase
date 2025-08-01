use crate::{
    instruction::{
        ed25519_edwards_decompress_validate::SyscallED25519EdwardsDecompressValidate,
        ed25519_ristretto_decompress_validate::SyscallED25519RistrettoDecompressValidate,
    },
    utils::syscall_process_exit_code,
    RuntimeContext,
};
use curve25519_dalek::{traits::MultiscalarMul, EdwardsPoint, RistrettoPoint, Scalar};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub const POINT_LEN: usize = 32;
pub const SCALAR_LEN: usize = 32;
const PAIR_LEN: usize = POINT_LEN + SCALAR_LEN;

pub(crate) struct SyscallED25519RistrettoMultiscalarMul {}

impl SyscallED25519RistrettoMultiscalarMul {
    pub const fn new() -> Self {
        Self {}
    }
}

impl SyscallED25519RistrettoMultiscalarMul {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (pairs_ptr, pairs_count, out_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
            params[2].i32().unwrap() as u32,
        );

        let pairs_byte_len = PAIR_LEN.saturating_mul(pairs_count as usize);

        let mut pair_elements = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pair_elements)?;

        let pairs: Result<Vec<([u8; 32], [u8; 32])>, ExitCode> = pair_elements
            .chunks(PAIR_LEN)
            .map(|v| {
                let point: [u8; POINT_LEN] =
                    unsafe { core::slice::from_raw_parts(v.as_ptr(), POINT_LEN) }
                        .try_into()
                        .map_err(|e| ExitCode::MalformedBuiltinParams)?;
                let scalar: [u8; SCALAR_LEN] = unsafe {
                    core::slice::from_raw_parts(
                        v[POINT_LEN..(POINT_LEN + SCALAR_LEN)].as_ptr(),
                        SCALAR_LEN,
                    )
                    .try_into()
                    .map_err(|e| ExitCode::MalformedBuiltinParams)?
                };
                Ok((point, scalar))
            })
            .collect();
        let pairs = match pairs {
            Ok(v) => v,
            Err(exit_code) => {
                return Err(syscall_process_exit_code(caller, exit_code));
            }
        };

        let res = Self::fn_impl(&pairs).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(res) = res {
            caller.memory_write(out_ptr as usize, res.compress().as_bytes())?;
        }
        result[0] = Value::I32(res.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(
        pairs: &[([u8; POINT_LEN], [u8; SCALAR_LEN])],
    ) -> Result<RistrettoPoint, ExitCode> {
        let points: Result<Vec<RistrettoPoint>, ExitCode> = pairs
            .iter()
            .map(|v| SyscallED25519RistrettoDecompressValidate::fn_impl(&v.0))
            .collect();
        let points = points?;
        let scalars: Vec<Scalar> = pairs
            .iter()
            .map(|v| Scalar::from_bytes_mod_order(v.1))
            .collect();
        let result = RistrettoPoint::multiscalar_mul(scalars, points);

        Ok(result)
    }
}

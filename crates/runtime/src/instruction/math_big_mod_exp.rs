use crate::{utils::syscall_process_exit_code, RuntimeContext};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallMathBigModExp {}

impl SyscallMathBigModExp {
    /// Create a new instance of the [`SyscallMathBigModExp`].
    pub const fn new() -> Self {
        Self {}
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (base_ptr, base_len, exp_ptr, exp_len, modulus_ptr, modulus_len) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
            params[2].i32().unwrap() as u32,
            params[3].i32().unwrap() as u32,
            params[4].i32().unwrap() as u32,
            params[5].i32().unwrap() as u32,
        );

        let mut base = vec![0u8; base_len as usize];
        caller.memory_read(base_ptr as usize, &mut base)?;
        let mut exp = vec![0u8; exp_len as usize];
        caller.memory_read(exp_ptr as usize, &mut exp)?;
        let mut modulus = vec![0u8; modulus_len as usize];
        caller.memory_read(modulus_ptr as usize, &mut modulus)?;

        // Write the result back to memory at the p_ptr location
        let res = Self::fn_impl(&base, &exp, &mut modulus);
        match res {
            Ok(_) => {
                caller.memory_write(modulus_ptr as usize, &modulus)?;
            }
            Err(e) => {
                syscall_process_exit_code(caller, e);
            }
        }
        result[0] = Value::I32(res.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode> {
        if base.len() <= 0 || exponent.len() <= 0 {
            return Err(ExitCode::MalformedBuiltinParams);
        }
        if modulus.len() <= 0 {
            return Ok(());
        }
        use num_bigint::BigUint;
        use num_traits::{One, Zero};

        let modulus_len = modulus.len();
        let base = BigUint::from_bytes_be(base);
        let exponent = BigUint::from_bytes_be(exponent);
        let modulus_uint = BigUint::from_bytes_be(modulus);

        if modulus_uint.is_zero() || modulus_uint.is_one() {
            modulus.fill(0);
            return Ok(());
        }

        let ret_int = base.modpow(&exponent, &modulus_uint);
        let ret_int = ret_int.to_bytes_be();
        let mut return_value = vec![0_u8; modulus_len.saturating_sub(ret_int.len())];
        return_value.extend(ret_int);
        modulus.copy_from_slice(&return_value);

        Ok(())
    }
}

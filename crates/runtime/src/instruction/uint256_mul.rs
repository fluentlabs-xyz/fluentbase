use crate::RuntimeContext;
use num::{BigUint, One, Zero};
use rwasm::{Caller, TrapCode, Value};
use sp1_curves::edwards::WORDS_FIELD_ELEMENT;

pub struct SyscallUint256Mul;

impl SyscallUint256Mul {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (x_ptr, y_ptr, m_ptr) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as usize,
            params[2].i32().unwrap() as usize,
        );

        let mut x = [0u8; WORDS_FIELD_ELEMENT];
        caller.memory_read(x_ptr, &mut x)?;
        let mut y = [0u8; WORDS_FIELD_ELEMENT];
        caller.memory_read(y_ptr, &mut y)?;
        let mut m = [0u8; WORDS_FIELD_ELEMENT];
        caller.memory_read(m_ptr, &mut m)?;

        let result_vec = Self::fn_impl(&x, &y, &m);
        caller.memory_write(x_ptr, &result_vec)
    }

    pub fn fn_impl(x: &[u8], y: &[u8], m: &[u8]) -> Vec<u8> {
        // Get the BigUint values for x, y, and the modulus.
        let uint256_x = BigUint::from_bytes_le(x);
        let uint256_y = BigUint::from_bytes_le(y);
        let uint256_m = BigUint::from_bytes_le(&m);

        // Perform the multiplication and take the result modulo the modulus.
        let result: BigUint = if uint256_m.is_zero() {
            let modulus = BigUint::one() << 256;
            (uint256_x * uint256_y) % modulus
        } else {
            (uint256_x * uint256_y) % uint256_m
        };

        let mut result_bytes = result.to_bytes_le();
        result_bytes.resize(32, 0u8); // Pad the result to 32 bytes.
        result_bytes
    }
}

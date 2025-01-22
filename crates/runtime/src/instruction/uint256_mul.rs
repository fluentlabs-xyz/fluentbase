use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use num::{BigUint, One, Zero};
use sp1_curves::edwards::WORDS_FIELD_ELEMENT;

pub struct SyscallUint256Mul;

impl SyscallUint256Mul {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [x_ptr, y_ptr, m_ptr] = caller.stack_pop_n();

        let x = caller.read_memory(x_ptr.as_u32(), WORDS_FIELD_ELEMENT as u32)?;
        let y = caller.read_memory(y_ptr.as_u32(), WORDS_FIELD_ELEMENT as u32)?;
        let m = caller.read_memory(m_ptr.as_u32(), WORDS_FIELD_ELEMENT as u32)?;

        let result_vec = Self::fn_impl(&x, &y, &m);
        caller.write_memory(x_ptr.as_u32(), &result_vec)
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

use crate::RuntimeContext;
use num::{BigUint, One, Zero};
use rwasm::{Store, TrapCode, Value};

pub fn syscall_uint256_mul_mod_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (x_ptr, y_ptr, m_ptr) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
    );

    let mut x = [0u8; 32];
    caller.memory_read(x_ptr, &mut x)?;
    let mut y = [0u8; 32];
    caller.memory_read(y_ptr, &mut y)?;
    let mut m = [0u8; 32];
    caller.memory_read(m_ptr, &mut m)?;

    let result_vec = syscall_uint256_mul_mod_impl(&x, &y, &m);
    caller.memory_write(x_ptr, &result_vec)
}

pub fn syscall_uint256_mul_mod_impl(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
    // Get the BigUint values for x, y, and the modulus.
    let uint256_x = BigUint::from_bytes_le(x);
    let uint256_y = BigUint::from_bytes_le(y);
    let uint256_m = BigUint::from_bytes_le(m);

    // Perform the multiplication and take the result modulo the modulus.
    let result: BigUint = if uint256_m.is_zero() {
        let modulus = BigUint::one() << 256;
        (uint256_x * uint256_y) % modulus
    } else {
        (uint256_x * uint256_y) % uint256_m
    };

    let mut result_bytes = result.to_bytes_le();
    result_bytes.resize(32, 0u8); // Pad the result to 32 bytes.
    let mut result = [0u8; 32];
    result.copy_from_slice(&result_bytes);
    result
}

/// These tests are taken from: sp1/crates/test-artifacts/programs/uint256-mul/src/main.rs
#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    fn biguint_to_bytes_le(x: BigUint) -> [u8; 32] {
        let mut bytes = x.to_bytes_le();
        bytes.resize(32, 0);
        bytes.try_into().unwrap()
    }

    #[test]
    fn test_u256_mul_mod() {
        for _ in 0..50 {
            // Test with random numbers.
            let mut rng = rand::rng();
            let mut x: [u8; 32] = rng.random();
            let mut y: [u8; 32] = rng.random();
            let modulus: [u8; 32] = rng.random();

            // Convert byte arrays to BigUint
            let modulus_big = BigUint::from_bytes_le(&modulus);
            let x_big = BigUint::from_bytes_le(&x);
            x = biguint_to_bytes_le(&x_big % &modulus_big);
            let y_big = BigUint::from_bytes_le(&y);
            y = biguint_to_bytes_le(&y_big % &modulus_big);

            let result_bytes = syscall_uint256_mul_mod_impl(&x, &y, &modulus);

            let result = (x_big * y_big) % modulus_big;
            let result_syscall = BigUint::from_bytes_le(&result_bytes);

            assert_eq!(result, result_syscall);
        }

        // Modulus zero tests
        let modulus = [0u8; 32];
        let modulus_big: BigUint = BigUint::one() << 256;
        for _ in 0..50 {
            // Test with random numbers.
            let mut rng = rand::rng();
            let mut x: [u8; 32] = rng.random();
            let mut y: [u8; 32] = rng.random();

            // Convert byte arrays to BigUint
            let x_big = BigUint::from_bytes_le(&x);
            x = biguint_to_bytes_le(&x_big % &modulus_big);
            let y_big = BigUint::from_bytes_le(&y);
            y = biguint_to_bytes_le(&y_big % &modulus_big);

            let result_bytes = syscall_uint256_mul_mod_impl(&x, &y, &modulus);

            let result = (x_big * y_big) % &modulus_big;
            let result_syscall = BigUint::from_bytes_le(&result_bytes);

            assert_eq!(result, result_syscall, "x: {:?}, y: {:?}", x, y);
        }

        // Test with random numbers.
        let mut rng = rand::rng();
        let x: [u8; 32] = rng.random();

        // Hardcoded edge case: Multiplying by 1
        let modulus = [0u8; 32];

        let mut one: [u8; 32] = [0; 32];
        one[0] = 1; // Least significant byte set to 1, represents the number 1
        let original_x = x; // Copy original x value before multiplication by 1
        let result_one = syscall_uint256_mul_mod_impl(&x, &one, &modulus);
        assert_eq!(
            result_one, original_x,
            "Multiplying by 1 should yield the same number."
        );

        // Hardcoded edge case: Multiplying by 0
        let zero: [u8; 32] = [0; 32]; // Represents the number 0
        let result_zero = syscall_uint256_mul_mod_impl(&x, &zero, &modulus);
        assert_eq!(result_zero, zero, "Multiplying by 0 should yield 0.");
    }
}

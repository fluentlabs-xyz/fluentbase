use crate::RuntimeContext;
use num::{BigUint, Integer, One};
use rwasm::{Store, TrapCode, Value};

const U256_NUM_BYTES: usize = 32;
const U2048_NUM_BYTES: usize = 256;

pub fn syscall_mul_x2048_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (a_ptr, b_ptr, lo_ptr, hi_ptr) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
        params[3].i32().unwrap() as usize,
    );

    let mut a = [0u8; U256_NUM_BYTES];
    ctx.memory_read(a_ptr, &mut a)?;
    let mut b = [0u8; U2048_NUM_BYTES];
    ctx.memory_read(b_ptr, &mut b)?;

    let (lo_bytes, hi_bytes) = syscall_x2048_mul_impl(a, b);

    ctx.memory_write(lo_ptr, &lo_bytes)?;
    ctx.memory_write(hi_ptr, &hi_bytes)?;
    Ok(())
}

pub fn syscall_x2048_mul_impl(
    a: [u8; U256_NUM_BYTES],
    b: [u8; U2048_NUM_BYTES],
) -> ([u8; U2048_NUM_BYTES], [u8; U256_NUM_BYTES]) {
    let uint256_a = BigUint::from_bytes_le(&a);
    let uint2048_b = BigUint::from_bytes_le(&b);
    let result = uint256_a * uint2048_b;
    let two_to_2048 = BigUint::one() << 2048;
    let (hi, lo) = result.div_rem(&two_to_2048);
    let mut lo_bytes = lo.to_bytes_le();
    lo_bytes.resize(U2048_NUM_BYTES, 0u8);
    let lo_res: [u8; U2048_NUM_BYTES] = lo_bytes.try_into().unwrap();
    let mut hi_bytes = hi.to_bytes_le();
    hi_bytes.resize(U256_NUM_BYTES, 0u8);
    let hi_res: [u8; U256_NUM_BYTES] = hi_bytes.try_into().unwrap();
    (lo_res, hi_res)
}

/// These tests are taken from: sp1/crates/test-artifacts/programs/u256x2048-mul/src/main.rs
#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{rand, rand::Rng};

    fn u256_to_bytes_le(x: &BigUint) -> [u8; 32] {
        let mut bytes = x.to_bytes_le();
        bytes.resize(32, 0);
        bytes.try_into().unwrap()
    }

    fn u2048_to_bytes_le(x: &BigUint) -> [u8; 256] {
        let mut bytes = x.to_bytes_le();
        bytes.resize(256, 0);
        bytes.try_into().unwrap()
    }

    #[test]
    fn test_uin256_x2048_mul_sp1() {
        let mut a_max: [u8; 32] = [0xff; 32];
        let mut b_max: [u8; 256] = [0xff; 256];

        let a_max_big = BigUint::from_bytes_le(&a_max);
        a_max = u256_to_bytes_le(&a_max_big);
        let b_max_big = BigUint::from_bytes_le(&b_max);
        b_max = u2048_to_bytes_le(&b_max_big);

        let (lo_max_bytes, hi_max_bytes) = syscall_x2048_mul_impl(a_max, b_max);

        let lo_max_big = BigUint::from_bytes_le(&lo_max_bytes);
        let hi_max_big = BigUint::from_bytes_le(&hi_max_bytes);

        let result_max_syscall = (hi_max_big << 2048) + lo_max_big;
        let result_max = a_max_big * b_max_big;
        assert_eq!(result_max, result_max_syscall);

        // Test 10 random pairs of a and b.
        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let a: [u8; 32] = rng.gen();
            let mut b = [0u8; 256];
            rng.fill(&mut b);

            let a_big = BigUint::from_bytes_le(&a);
            let b_big = BigUint::from_bytes_le(&b);

            let a = u256_to_bytes_le(&a_big);
            let b = u2048_to_bytes_le(&b_big);

            let (lo_bytes, hi_bytes) = syscall_x2048_mul_impl(a, b);

            let lo_big = BigUint::from_bytes_le(&lo_bytes);
            let hi_big = BigUint::from_bytes_le(&hi_bytes);

            let result_syscall = (hi_big << 2048) + lo_big;
            let result = a_big * b_big;
            assert_eq!(result, result_syscall);
        }

        println!("All tests passed successfully!");
    }
}

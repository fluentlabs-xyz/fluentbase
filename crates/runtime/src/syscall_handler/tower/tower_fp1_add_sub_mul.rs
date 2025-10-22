use crate::RuntimeContext;
use fluentbase_types::{BLS12381_FP_SIZE, BN254_FP_SIZE};
use num::BigUint;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::weierstrass::{bls12_381::Bls12381BaseField, bn254::Bn254BaseField, FpOpField};

pub fn syscall_tower_fp1_bn254_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_ADD>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp1_bn254_sub_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_SUB>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp1_bn254_mul_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_MUL>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp1_bls12381_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_ADD>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp1_bls12381_sub_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_SUB>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp1_bls12381_mul_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp1_add_sub_mul_handler::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_MUL>(
        ctx, params, _result,
    )
}

const FP_FIELD_ADD: u32 = 0x01;
const FP_FIELD_SUB: u32 = 0x02;
const FP_FIELD_MUL: u32 = 0x03;

pub(crate) fn syscall_tower_fp1_add_sub_mul_handler<
    const NUM_BYTES: usize,
    P: FpOpField,
    const FIELD_OP: u32,
>(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (x_ptr, y_ptr) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as u32,
    );
    let mut x = [0u8; NUM_BYTES];
    ctx.memory_read(x_ptr as usize, &mut x)?;
    let mut y = [0u8; NUM_BYTES];
    ctx.memory_read(y_ptr as usize, &mut y)?;

    let result = syscall_tower_fp1_add_sub_mul_impl::<NUM_BYTES, P, FIELD_OP>(
        x.try_into().unwrap(),
        y.try_into().unwrap(),
    );

    ctx.memory_write(x_ptr as usize, &result)?;
    Ok(())
}

pub fn syscall_tower_fp1_bn254_add_impl(
    x: [u8; BN254_FP_SIZE],
    y: [u8; BN254_FP_SIZE],
) -> [u8; BN254_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_ADD>(x, y)
}
pub fn syscall_tower_fp1_bn254_sub_impl(
    x: [u8; BN254_FP_SIZE],
    y: [u8; BN254_FP_SIZE],
) -> [u8; BN254_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_SUB>(x, y)
}
pub fn syscall_tower_fp1_bn254_mul_impl(
    x: [u8; BN254_FP_SIZE],
    y: [u8; BN254_FP_SIZE],
) -> [u8; BN254_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BN254_FP_SIZE, Bn254BaseField, FP_FIELD_MUL>(x, y)
}
pub fn syscall_tower_fp1_bls12381_add_impl(
    x: [u8; BLS12381_FP_SIZE],
    y: [u8; BLS12381_FP_SIZE],
) -> [u8; BLS12381_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_ADD>(x, y)
}
pub fn syscall_tower_fp1_bls12381_sub_impl(
    x: [u8; BLS12381_FP_SIZE],
    y: [u8; BLS12381_FP_SIZE],
) -> [u8; BLS12381_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_SUB>(x, y)
}
pub fn syscall_tower_fp1_bls12381_mul_impl(
    x: [u8; BLS12381_FP_SIZE],
    y: [u8; BLS12381_FP_SIZE],
) -> [u8; BLS12381_FP_SIZE] {
    syscall_tower_fp1_add_sub_mul_impl::<BLS12381_FP_SIZE, Bls12381BaseField, FP_FIELD_MUL>(x, y)
}

pub(crate) fn syscall_tower_fp1_add_sub_mul_impl<
    const NUM_BYTES: usize,
    P: FpOpField,
    const FIELD_OP: u32,
>(
    x: [u8; NUM_BYTES],
    y: [u8; NUM_BYTES],
) -> [u8; NUM_BYTES] {
    let modulus = &BigUint::from_bytes_le(P::MODULUS);
    let a = BigUint::from_bytes_le(&x) % modulus;
    let b = BigUint::from_bytes_le(&y) % modulus;
    let result = match FIELD_OP {
        FP_FIELD_ADD => (a + b) % modulus,
        FP_FIELD_SUB => {
            // TODO(dmitry123): Due to the SP1 limitations, we can't support b that is greater than a + modulus.
            //  But what's the best workaround here? To return an error or to wrap b?
            ((a + modulus) - b % modulus) % modulus
        }
        FP_FIELD_MUL => (a * b) % modulus,
        _ => unreachable!(),
    };
    let mut result = result.to_bytes_le();
    result.resize(NUM_BYTES, 0);
    result.try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;
    use std::str::FromStr;

    fn random_bigint<const NUM_BYTES: usize>(modulus: &BigUint) -> BigUint {
        let mut rng = rand::rng();
        let mut arr = vec![0u8; NUM_BYTES];
        for item in arr.iter_mut() {
            *item = rng.random();
        }
        BigUint::from_bytes_le(&arr) % modulus
    }

    fn big_uint_into_bytes<const NUM_BYTES: usize>(a: &BigUint) -> [u8; NUM_BYTES] {
        let mut res = a.to_bytes_le();
        res.resize(NUM_BYTES, 0);
        res.try_into().unwrap()
    }

    /// Tests are stolen from: sp1/crates/test-artifacts/programs/bn254-fp/src/main.rs
    #[test]
    fn test_bn254_fp1() {
        let modulus = BigUint::from_str(
            "21888242871839275222246405745257275088696311157297823662689037894645226208583",
        )
        .unwrap();
        let zero = BigUint::ZERO;
        let one = BigUint::from(1u32);

        let add = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bn254_add_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };
        let sub = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bn254_sub_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };
        let mul = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bn254_mul_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };

        for _ in 0..10 {
            let a = random_bigint::<32>(&modulus);
            let b = random_bigint::<32>(&modulus);

            // Test addition
            let result = add(&a, &b) % &modulus;
            assert_eq!((&a + &b) % &modulus, result);

            // Test addition with zero
            let result = add(&a, &zero) % &modulus;
            assert_eq!((&a + &zero) % &modulus, result);

            // Test subtraction
            let expected_sub = if a < b {
                ((&a + &modulus) - &b) % &modulus
            } else {
                (&a - &b) % &modulus
            };
            let result = sub(&a, &b) % &modulus;
            assert_eq!(expected_sub, result);

            // Test subtraction with zero
            let result = sub(&a, &zero) % &modulus;
            assert_eq!((&a + &modulus - &zero) % &modulus, result);

            // Test multiplication
            let result = mul(&a, &b) % &modulus;
            assert_eq!((&a * &b) % &modulus, result);

            // Test multiplication with one
            let result = mul(&a, &one) % &modulus;
            assert_eq!((&a * &one) % &modulus, result);

            // Test multiplication with zero
            let result = mul(&a, &zero) % &modulus;
            assert_eq!((&a * &zero) % &modulus, result);
        }
    }

    /// Tests are stolen from: sp1/crates/test-artifacts/programs/bls12381-fp/src/main.rs
    #[test]
    fn test_bls12381_fp1() {
        let modulus = BigUint::from_str("4002409555221667393417789825735904156556882819939007885332058136124031650490837864442687629129015664037894272559787").unwrap();
        let zero = BigUint::ZERO;
        let one = BigUint::from(1u32);

        let add = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bls12381_add_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };
        let sub = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bls12381_sub_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };
        let mul = |a: &BigUint, b: &BigUint| -> BigUint {
            let result =
                syscall_tower_fp1_bls12381_mul_impl(big_uint_into_bytes(a), big_uint_into_bytes(b));
            BigUint::from_bytes_le(&result)
        };

        for _ in 0..10 {
            let a = random_bigint::<48>(&modulus);
            let b = random_bigint::<48>(&modulus);

            // Test addition
            let result = add(&a, &b) % &modulus;
            assert_eq!((&a + &b) % &modulus, result);

            // Test addition with zero
            let result = add(&a, &zero) % &modulus;
            assert_eq!((&a + &zero) % &modulus, result);

            // Test subtraction
            let expected_sub = if a < b {
                ((&a + &modulus) - &b) % &modulus
            } else {
                (&a - &b) % &modulus
            };
            let result = sub(&a, &b) % &modulus;
            assert_eq!(expected_sub, result);

            // Test subtraction with zero
            let result = sub(&a, &zero) % &modulus;
            assert_eq!((&a + &modulus - &zero) % &modulus, result);

            // Test multiplication
            let result = mul(&a, &b) % &modulus;
            assert_eq!((&a * &b) % &modulus, result);

            // Test multiplication with one
            let result = mul(&a, &one) % &modulus;
            assert_eq!((&a * &one) % &modulus, result);

            // Test multiplication with zero
            let result = &mul(&a, &zero) % &modulus;
            assert_eq!((&a * &zero) % &modulus, result,);
        }
    }

    #[test]
    fn test_bls12381_overflow_cant_happen() {
        let m = BigUint::from_str("4002409555221667393417789825735904156556882819939007885332058136124031650490837864442687629129015664037894272559787").unwrap();
        let a = BigUint::from_str("3001807166416250545063342369301928117417662114954255913999043602093023737868128398332015721846761748028420704419841").unwrap();
        let b = BigUint::from_str("12007228665665002180253369477207712469670648459817023655996174408372094951472513593328062887387046992113682817679361").unwrap();
        assert!(a < m && b > m);

        let result =
            syscall_tower_fp1_bls12381_sub_impl(big_uint_into_bytes(&a), big_uint_into_bytes(&b));
        let result = BigUint::from_bytes_le(&result);

        let expected_sub = ((&a + &m) - &b % &m) % &m;
        assert_eq!(expected_sub, result);
    }
}

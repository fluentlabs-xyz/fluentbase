use crate::RuntimeContext;
use num::BigUint;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::weierstrass::{bls12_381::Bls12381BaseField, bn254::Bn254BaseField, FpOpField};

pub fn syscall_tower_fp2_bn254_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<32, Bn254BaseField, FP_FIELD_ADD>(ctx, params, _result)
}
pub fn syscall_tower_fp2_bn254_sub_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<32, Bn254BaseField, FP_FIELD_SUB>(ctx, params, _result)
}
pub fn syscall_tower_fp2_bn254_mul_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<32, Bn254BaseField, FP_FIELD_MUL>(ctx, params, _result)
}
pub fn syscall_tower_fp2_bls12381_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<48, Bls12381BaseField, FP_FIELD_ADD>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp2_bls12381_sub_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<48, Bls12381BaseField, FP_FIELD_SUB>(
        ctx, params, _result,
    )
}
pub fn syscall_tower_fp2_bls12381_mul_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_tower_fp2_add_sub_mul_handler::<48, Bls12381BaseField, FP_FIELD_MUL>(
        ctx, params, _result,
    )
}

const FP_FIELD_ADD: u32 = 0x01;
const FP_FIELD_SUB: u32 = 0x02;
const FP_FIELD_MUL: u32 = 0x03;

pub(crate) fn syscall_tower_fp2_add_sub_mul_handler<
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
    let mut ac0 = [0u8; NUM_BYTES];
    let mut ac1 = [0u8; NUM_BYTES];
    ctx.memory_read(x_ptr as usize, &mut ac0)?;
    ctx.memory_read(x_ptr as usize + NUM_BYTES, &mut ac1)?;
    let mut bc0 = [0u8; NUM_BYTES];
    let mut bc1 = [0u8; NUM_BYTES];
    ctx.memory_read(y_ptr as usize, &mut bc0)?;
    ctx.memory_read(y_ptr as usize + NUM_BYTES, &mut bc1)?;

    let (res0, res1) = syscall_tower_fp2_add_sub_mul_impl::<NUM_BYTES, P, FIELD_OP>(
        ac0.try_into().unwrap(),
        ac1.try_into().unwrap(),
        bc0.try_into().unwrap(),
        bc1.try_into().unwrap(),
    );

    ctx.memory_write(x_ptr as usize, &res0)?;
    ctx.memory_write(x_ptr as usize + NUM_BYTES, &res1)?;
    Ok(())
}

pub fn syscall_tower_fp2_bn254_add_impl(
    ac0: [u8; 32],
    ac1: [u8; 32],
    bc0: [u8; 32],
    bc1: [u8; 32],
) -> ([u8; 32], [u8; 32]) {
    syscall_tower_fp2_add_sub_mul_impl::<32, Bn254BaseField, FP_FIELD_ADD>(ac0, ac1, bc0, bc1)
}
pub fn syscall_tower_fp2_bn254_sub_impl(
    ac0: [u8; 32],
    ac1: [u8; 32],
    bc0: [u8; 32],
    bc1: [u8; 32],
) -> ([u8; 32], [u8; 32]) {
    syscall_tower_fp2_add_sub_mul_impl::<32, Bn254BaseField, FP_FIELD_SUB>(ac0, ac1, bc0, bc1)
}
pub fn syscall_tower_fp2_bn254_mul_impl(
    ac0: [u8; 32],
    ac1: [u8; 32],
    bc0: [u8; 32],
    bc1: [u8; 32],
) -> ([u8; 32], [u8; 32]) {
    syscall_tower_fp2_add_sub_mul_impl::<32, Bn254BaseField, FP_FIELD_MUL>(ac0, ac1, bc0, bc1)
}
pub fn syscall_tower_fp2_bls12381_add_impl(
    ac0: [u8; 48],
    ac1: [u8; 48],
    bc0: [u8; 48],
    bc1: [u8; 48],
) -> ([u8; 48], [u8; 48]) {
    syscall_tower_fp2_add_sub_mul_impl::<48, Bls12381BaseField, FP_FIELD_ADD>(ac0, ac1, bc0, bc1)
}
pub fn syscall_tower_fp2_bls12381_sub_impl(
    ac0: [u8; 48],
    ac1: [u8; 48],
    bc0: [u8; 48],
    bc1: [u8; 48],
) -> ([u8; 48], [u8; 48]) {
    syscall_tower_fp2_add_sub_mul_impl::<48, Bls12381BaseField, FP_FIELD_SUB>(ac0, ac1, bc0, bc1)
}
pub fn syscall_tower_fp2_bls12381_mul_impl(
    ac0: [u8; 48],
    ac1: [u8; 48],
    bc0: [u8; 48],
    bc1: [u8; 48],
) -> ([u8; 48], [u8; 48]) {
    syscall_tower_fp2_add_sub_mul_impl::<48, Bls12381BaseField, FP_FIELD_MUL>(ac0, ac1, bc0, bc1)
}

pub(crate) fn syscall_tower_fp2_add_sub_mul_impl<
    const NUM_BYTES: usize,
    P: FpOpField,
    const FIELD_OP: u32,
>(
    ac0: [u8; NUM_BYTES],
    ac1: [u8; NUM_BYTES],
    bc0: [u8; NUM_BYTES],
    bc1: [u8; NUM_BYTES],
) -> ([u8; NUM_BYTES], [u8; NUM_BYTES]) {
    let ac0 = &BigUint::from_bytes_le(&ac0);
    let ac1 = &BigUint::from_bytes_le(&ac1);
    let bc0 = &BigUint::from_bytes_le(&bc0);
    let bc1 = &BigUint::from_bytes_le(&bc1);
    let modulus = &BigUint::from_bytes_le(P::MODULUS);
    let (c0, c1) = match FIELD_OP {
        FP_FIELD_ADD => ((ac0 + bc0) % modulus, (ac1 + bc1) % modulus),
        FP_FIELD_SUB => (
            (ac0 + modulus - bc0) % modulus,
            (ac1 + modulus - bc1) % modulus,
        ),
        FP_FIELD_MUL => {
            let c0 = match (ac0 * bc0) % modulus < (ac1 * bc1) % modulus {
                true => ((modulus + (ac0 * bc0) % modulus) - (ac1 * bc1) % modulus) % modulus,
                false => ((ac0 * bc0) % modulus - (ac1 * bc1) % modulus) % modulus,
            };
            let c1 = ((ac0 * bc1) % modulus + (ac1 * bc0) % modulus) % modulus;
            (c0, c1)
        }
        _ => unreachable!(),
    };
    let mut res0 = c0.to_bytes_le();
    res0.resize(NUM_BYTES, 0);
    let mut res1 = c1.to_bytes_le();
    res1.resize(NUM_BYTES, 0);
    (res0.try_into().unwrap(), res1.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{rand, rand::Rng};
    use std::str::FromStr;

    fn random_bigint<const NUM_BYTES: usize>(modulus: &BigUint) -> BigUint {
        let mut rng = rand::thread_rng();
        let mut arr = vec![0u8; NUM_BYTES];
        for item in arr.iter_mut() {
            *item = rng.gen();
        }
        BigUint::from_bytes_le(&arr) % modulus
    }

    fn big_uint_into_bytes<const NUM_BYTES: usize>(a: &BigUint) -> [u8; NUM_BYTES] {
        let mut res = a.to_bytes_le();
        res.resize(NUM_BYTES, 0);
        res.try_into().unwrap()
    }

    #[test]
    fn test_tower_fp2_bn254_add_sub_mul() {
        const MODULUS: &str =
            "21888242871839275222246405745257275088696311157297823662689037894645226208583";
        let modulus = BigUint::from_str(MODULUS).unwrap();

        let add =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let bytes = ac0.to_bytes_le();
                let (res0, res1) = syscall_tower_fp2_bn254_add_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };
        let sub =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let (res0, res1) = syscall_tower_fp2_bn254_sub_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };
        let mul =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let (res0, res1) = syscall_tower_fp2_bn254_mul_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };

        let (zero0, zero1) = add(
            &BigUint::ZERO,
            &BigUint::ZERO,
            &BigUint::ZERO,
            &BigUint::ZERO,
        );
        assert_eq!(zero0, BigUint::ZERO);
        assert_eq!(zero1, BigUint::ZERO);

        for _ in 0..10 {
            let ac0 = random_bigint::<32>(&modulus);
            let ac1 = random_bigint::<32>(&modulus);
            let bc0 = random_bigint::<32>(&modulus);
            let bc1 = random_bigint::<32>(&modulus);

            // Fp2 Addition test
            let c0 = (&ac0 + &bc0) % &modulus;
            let c1 = (&ac1 + &bc1) % &modulus;

            let (res_c0, res_c1) = add(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);

            // Fp2 Subtraction test
            let c0 = (&ac0 + &modulus - &bc0) % &modulus;
            let c1 = (&ac1 + &modulus - &bc1) % &modulus;

            let (res_c0, res_c1) = sub(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);
        }

        for _ in 0..10 {
            let ac0 = random_bigint::<32>(&modulus);
            let ac1 = random_bigint::<32>(&modulus);
            let bc0 = random_bigint::<32>(&modulus);
            let bc1 = random_bigint::<32>(&modulus);

            let ac0_bc0_mod = (&ac0 * &bc0) % &modulus;
            let ac1_bc1_mod = (&ac1 * &bc1) % &modulus;

            let c0 = if ac0_bc0_mod < ac1_bc1_mod {
                (&modulus + ac0_bc0_mod - ac1_bc1_mod) % &modulus
            } else {
                (ac0_bc0_mod - ac1_bc1_mod) % &modulus
            };

            let c1 = ((&ac0 * &bc1) % &modulus + (&ac1 * &bc0) % &modulus) % &modulus;

            let (res_c0, res_c1) = mul(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);
        }
    }

    #[test]
    fn test_tower_fp2_bls12381_add_sub_mul() {
        const MODULUS: &str =
            "4002409555221667393417789825735904156556882819939007885332058136124031650490837864442687629129015664037894272559787";
        let modulus = BigUint::from_str(MODULUS).unwrap();

        let add =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let bytes = ac0.to_bytes_le();
                let (res0, res1) = syscall_tower_fp2_bls12381_add_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };
        let sub =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let (res0, res1) = syscall_tower_fp2_bls12381_sub_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };
        let mul =
            |ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint| -> (BigUint, BigUint) {
                let (res0, res1) = syscall_tower_fp2_bls12381_mul_impl(
                    big_uint_into_bytes(ac0),
                    big_uint_into_bytes(ac1),
                    big_uint_into_bytes(bc0),
                    big_uint_into_bytes(bc1),
                );
                (BigUint::from_bytes_le(&res0), BigUint::from_bytes_le(&res1))
            };

        let (zero0, zero1) = add(
            &BigUint::ZERO,
            &BigUint::ZERO,
            &BigUint::ZERO,
            &BigUint::ZERO,
        );
        assert_eq!(zero0, BigUint::ZERO);
        assert_eq!(zero1, BigUint::ZERO);

        for _ in 0..10 {
            let ac0 = random_bigint::<48>(&modulus);
            let ac1 = random_bigint::<48>(&modulus);
            let bc0 = random_bigint::<48>(&modulus);
            let bc1 = random_bigint::<48>(&modulus);

            // Fp2 Addition test
            let c0 = (&ac0 + &bc0) % &modulus;
            let c1 = (&ac1 + &bc1) % &modulus;

            let (res_c0, res_c1) = add(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);

            // Fp2 Subtraction test
            let c0 = (&ac0 + &modulus - &bc0) % &modulus;
            let c1 = (&ac1 + &modulus - &bc1) % &modulus;

            let (res_c0, res_c1) = sub(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);
        }

        for _ in 0..10 {
            let ac0 = random_bigint::<48>(&modulus);
            let ac1 = random_bigint::<48>(&modulus);
            let bc0 = random_bigint::<48>(&modulus);
            let bc1 = random_bigint::<48>(&modulus);

            let ac0_bc0_mod = (&ac0 * &bc0) % &modulus;
            let ac1_bc1_mod = (&ac1 * &bc1) % &modulus;

            let c0 = if ac0_bc0_mod < ac1_bc1_mod {
                (&modulus + ac0_bc0_mod - ac1_bc1_mod) % &modulus
            } else {
                (ac0_bc0_mod - ac1_bc1_mod) % &modulus
            };

            let c1 = ((&ac0 * &bc1) % &modulus + (&ac1 * &bc0) % &modulus) % &modulus;

            let (res_c0, res_c1) = mul(&ac0, &ac1, &bc0, &bc1);

            assert_eq!(c0, &res_c0 % &modulus);
            assert_eq!(c1, &res_c1 % &modulus);
        }
    }
}

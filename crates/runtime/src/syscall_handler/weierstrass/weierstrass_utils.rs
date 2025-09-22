use num_bigint::BigUint;

/// Field operation over a prime field used by FP builtins.
pub trait FieldOp {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint;
}

/// Binary operation on quadratic extension field elements (a0 + a1*i) and (b0 + b1*i).
pub trait FieldOp2 {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint);
}

pub struct FieldAdd;
impl FieldOp for FieldAdd {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a + b) % modulus
    }
}

impl FieldOp2 for FieldAdd {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint) {
        ((ac0 + bc0) % modulus, (ac1 + bc1) % modulus)
    }
}

pub struct FieldMul;
impl FieldOp for FieldMul {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a * b) % modulus
    }
}

pub struct FieldSub;
impl FieldOp for FieldSub {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        ((a + modulus) - b) % modulus
    }
}

impl FieldOp2 for FieldSub {
    fn execute(
        ac0: &BigUint,
        ac1: &BigUint,
        bc0: &BigUint,
        bc1: &BigUint,
        modulus: &BigUint,
    ) -> (BigUint, BigUint) {
        (
            (ac0 + modulus - bc0) % modulus,
            (ac1 + modulus - bc1) % modulus,
        )
    }
}

/// Interprets a byte slice as a u32 slice if length is a multiple of 4; otherwise returns None.
pub fn cast_u8_to_u32(slice: &[u8]) -> Option<&[u32]> {
    if slice.len() % 4 != 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u32, slice.len() / 4) })
}

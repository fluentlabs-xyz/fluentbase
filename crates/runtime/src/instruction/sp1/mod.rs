use serde::{Deserialize, Serialize};
use num::BigUint;


pub(crate) mod keccak_permute;
pub(crate) mod uint256_mul;
pub(crate) mod halt;
pub(crate) mod write;
pub(crate) mod sha256_extend;
pub(crate) mod sha256_compress;
pub(crate) mod ed_decompress;
pub(crate) mod ed_add;
pub(crate) mod weierstrass_add;
pub(crate) mod weierstrass_double;
pub(crate) mod weierstrass_decompress;
pub(crate) mod fp_op;
pub(crate) mod fp2_addsub;
pub(crate) mod fp2_mul;

fn cast_u8_to_u32(slice: &[u8]) -> Option<&[u32]> {

    if slice.len() % 4 != 0 {
        return None;
    }

    Some(unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u32,
            slice.len() / 4
        )
    })
}

pub trait FieldOp {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint;
}

pub trait FieldOp2 {
    fn execute(ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint, modulus: &BigUint) -> (BigUint, BigUint);
}

pub struct Add;
impl FieldOp for Add{
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a + b) % modulus
    }
}

impl FieldOp2 for Add{
    fn execute(ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint, modulus: &BigUint) -> (BigUint, BigUint)  {
        ((ac0 + bc0) % modulus, (ac1 + bc1) % modulus)
    }
}

pub struct Mul;
impl FieldOp for Mul {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        (a * b) % modulus
    }
}

pub struct Sub;
impl FieldOp for Sub {
    fn execute(a: BigUint, b: BigUint, modulus: &BigUint) -> BigUint {
        ((a + modulus) - b) % modulus
    }
}

impl FieldOp2 for Sub{
    fn execute(ac0: &BigUint, ac1: &BigUint, bc0: &BigUint, bc1: &BigUint, modulus: &BigUint) -> (BigUint, BigUint) {
        ((ac0 + modulus - bc0) % modulus, (ac1 + modulus - bc1) % modulus)
    }
}

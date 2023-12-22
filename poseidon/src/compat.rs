use ff::*;

impl<T> FieldExt for T
 where
   T: ff::WithSmallOrderMulGroup<3>,
   T: Ord + From<bool>,
{

    /// Modulus of the field written as a string for display purposes
    const MODULUS: &'static str = <T as ff::PrimeField>::MODULUS;

    /// Inverse of `PrimeField::root_of_unity()`
    const ROOT_OF_UNITY_INV: Self = <T as ff::PrimeField>::ROOT_OF_UNITY_INV;

    /// Generator of the $t-order$ multiplicative subgroup
    const DELTA: Self = <T as ff::PrimeField>::DELTA;

    /// Inverse of $2$ in the field.
    const TWO_INV: Self = <T as ff::PrimeField>::TWO_INV;

    /// Element of multiplicative order $3$.
    const ZETA: Self = <T as ff::WithSmallOrderMulGroup<3>>::ZETA;

    //fn from_u128(v: u128) -> Self { <T as ff::PrimeField>::from_u128(v) }

    /// Obtains a field element that is congruent to the provided little endian
    /// byte representation of an integer.
    fn from_bytes_wide(bytes: &[u8; 64]) -> Self { unimplemented!() }

    /// Exponentiates `self` by `by`, where `by` is a little-endian order
    /// integer exponent.
    fn pow(&self, by: &[u64; 4]) -> Self {
        let mut res = Self::ONE;
        for e in by.iter().rev() {
            for i in (0..64).rev() {
                res = res.square();
                let mut tmp = res;
                tmp *= self;
                res.conditional_assign(&tmp, (((*e >> i) & 0x1) as u8).into());
            }
        }
        res
    }

    /// Gets the lower 128 bits of this field element when expressed
    /// canonically.
    fn get_lower_128(&self) -> u128 { unimplemented!() }

    fn zero() -> Self { Self::ZERO }
    fn one() -> Self { Self::ZERO }

}

/// This trait is a common interface for dealing with elements of a finite
/// field.
pub trait FieldExt: From<bool> + Ord + ff::WithSmallOrderMulGroup<3> {

    /// Modulus of the field written as a string for display purposes
    const MODULUS: &'static str;

    /// Inverse of `PrimeField::root_of_unity()`
    const ROOT_OF_UNITY_INV: Self;

    /// Generator of the $t-order$ multiplicative subgroup
    const DELTA: Self;

    /// Inverse of $2$ in the field.
    const TWO_INV: Self;

    /// Element of multiplicative order $3$.
    const ZETA: Self;

    //fn from_u128(v: u128) -> Self;

    /// Obtains a field element that is congruent to the provided little endian
    /// byte representation of an integer.
    fn from_bytes_wide(bytes: &[u8; 64]) -> Self;

    /// Exponentiates `self` by `by`, where `by` is a little-endian order
    /// integer exponent.
    fn pow(&self, by: &[u64; 4]) -> Self {
        let mut res = Self::ONE;
        for e in by.iter().rev() {
            for i in (0..64).rev() {
                res = res.square();
                let mut tmp = res;
                tmp *= self;
                res.conditional_assign(&tmp, (((*e >> i) & 0x1) as u8).into());
            }
        }
        res
    }

    /// Gets the lower 128 bits of this field element when expressed
    /// canonically.
    fn get_lower_128(&self) -> u128;

    fn zero() -> Self;
    fn one() -> Self;

}


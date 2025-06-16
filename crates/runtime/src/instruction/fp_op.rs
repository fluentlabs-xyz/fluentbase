use crate::{
    instruction::{cast_u8_to_u32, FieldOp},
    RuntimeContext,
};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use num::BigUint;
use rwasm::{Caller, TrapCode};
use sp1_curves::{params::NumWords, weierstrass::FpOpField};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallFpOp<P, OP> {
    _op: PhantomData<OP>,
    _marker: PhantomData<P>,
}

impl<P: FpOpField, OP: FieldOp> SyscallFpOp<P, OP> {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let (x_ptr, y_ptr) = caller.stack_pop2_as::<u32>();

        let num_words = <P as NumWords>::WordsFieldElement::USIZE;

        let x = caller.memory_read_vec(x_ptr as usize, num_words * 4)?;
        let y = caller.memory_read_vec(y_ptr as usize, num_words * 4)?;

        let result_vec = Self::fn_impl(&x, &y);
        caller.memory_write(x_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(x: &[u8], y: &[u8]) -> Vec<u8> {
        let num_words = <P as NumWords>::WordsFieldElement::USIZE;

        let x = cast_u8_to_u32(x).unwrap();
        let y = cast_u8_to_u32(y).unwrap();

        let modulus = &BigUint::from_bytes_le(P::MODULUS);
        let a = BigUint::from_slice(&x) % modulus;
        let b = BigUint::from_slice(&y) % modulus;

        let result = OP::execute(a, b, modulus);

        let mut result = result.to_u32_digits();
        result.resize(num_words, 0);

        words_to_bytes_le_vec(result.as_slice())
    }
}

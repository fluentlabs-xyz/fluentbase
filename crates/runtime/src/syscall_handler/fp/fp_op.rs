use crate::{
    syscall_handler::{cast_u8_to_u32, FieldOp},
    RuntimeContext,
};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use num::BigUint;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{params::NumWords, weierstrass::FpOpField};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallEccFpOp<P, OP> {
    _op: PhantomData<OP>,
    _marker: PhantomData<P>,
}

impl<P: FpOpField, OP: FieldOp> SyscallEccFpOp<P, OP> {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (x_ptr, y_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );

        let num_words = <P as NumWords>::WordsFieldElement::USIZE;

        let mut x = vec![0u8; num_words * 4];
        caller.memory_read(x_ptr as usize, &mut x)?;
        let mut y = vec![0u8; num_words * 4];
        caller.memory_read(y_ptr as usize, &mut y)?;

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

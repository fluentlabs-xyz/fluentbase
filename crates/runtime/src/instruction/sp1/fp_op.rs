use num::BigUint;
use sp1_curves::{
    params::NumWords,
    weierstrass::{FieldType, FpOpField},
};
use std::marker::PhantomData;
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::Caller;
use rwasm::core::Trap;
use sp1_primitives::consts::words_to_bytes_le_vec;
use serde::{Deserialize, Serialize};


use crate::{RuntimeContext};
use crate::instruction::sp1::{cast_u8_to_u32, FieldOperation};

pub struct SyscallFpOp<P> {
    op: FieldOperation,
    _marker: PhantomData<P>,
}

impl<P> SyscallFpOp<P> {
    pub const fn new(op: FieldOperation) -> Self {
        Self { op, _marker: PhantomData }
    }
}

impl<P: FpOpField> SyscallFpOp<P> {
    fn fn_handler(&self, mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
        let x_ptr = arg1;
        if x_ptr % 4 != 0 {
            panic!();
        }
        let y_ptr = arg2;
        if y_ptr % 4 != 0 {
            panic!();
        }

        let num_words = <P as NumWords>::WordsFieldElement::USIZE;

        let x = caller.read_memory(x_ptr, num_words as u32 * 4)?;
        let y = caller.read_memory(y_ptr, num_words as u32 * 4)?;

        let x = cast_u8_to_u32(x).unwrap();
        let y = cast_u8_to_u32(y).unwrap();

        let modulus = &BigUint::from_bytes_le(P::MODULUS);
        let a = BigUint::from_slice(&x) % modulus;
        let b = BigUint::from_slice(&y) % modulus;

        let result = match self.op {
            FieldOperation::Add => (a + b) % modulus,
            FieldOperation::Sub => ((a + modulus) - b) % modulus,
            FieldOperation::Mul => (a * b) % modulus,
            _ => panic!("Unsupported operation"),
        };
        let mut result = result.to_u32_digits();
        result.resize(num_words, 0);

        caller.write_memory(x_ptr, &words_to_bytes_le_vec(result.as_slice()))?;

        Ok(())
    }
}

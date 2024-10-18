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
use crate::instruction::sp1::{cast_u8_to_u32};


pub struct SyscallFp2Mul<P> {
    _marker: PhantomData<P>,
}

impl<P: FpOpField> SyscallFp2Mul<P> {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
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

        let (ac0, ac1) = x.split_at(x.len() / 2);
        let (bc0, bc1) = y.split_at(y.len() / 2);

        let ac0 = &BigUint::from_slice(ac0);
        let ac1 = &BigUint::from_slice(ac1);
        let bc0 = &BigUint::from_slice(bc0);
        let bc1 = &BigUint::from_slice(bc1);
        let modulus = &BigUint::from_bytes_le(P::MODULUS);

        #[allow(clippy::match_bool)]
        let c0 = match (ac0 * bc0) % modulus < (ac1 * bc1) % modulus {
            true => ((modulus + (ac0 * bc0) % modulus) - (ac1 * bc1) % modulus) % modulus,
            false => ((ac0 * bc0) % modulus - (ac1 * bc1) % modulus) % modulus,
        };
        let c1 = ((ac0 * bc1) % modulus + (ac1 * bc0) % modulus) % modulus;

        let mut result =
            c0.to_u32_digits().into_iter().chain(c1.to_u32_digits()).collect::<Vec<u32>>();
        result.resize(num_words, 0);

        caller.write_memory(x_ptr, &words_to_bytes_le_vec(result.as_slice()))?;

        Ok(())
    }
}

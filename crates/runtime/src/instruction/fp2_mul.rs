use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use num::BigUint;
use rwasm::{Caller, TrapCode};
use sp1_curves::{params::NumWords, weierstrass::FpOpField};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallFp2Mul<P> {
    _marker: PhantomData<P>,
}

impl<P: FpOpField> SyscallFp2Mul<P> {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
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

        let mut result = c0
            .to_u32_digits()
            .into_iter()
            .chain(c1.to_u32_digits())
            .collect::<Vec<u32>>();
        result.resize(num_words, 0);

        words_to_bytes_le_vec(result.as_slice())
    }
}

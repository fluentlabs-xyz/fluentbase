use std::marker::PhantomData;
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::Caller;
use rwasm::core::Trap;
use sp1_curves::{edwards::EdwardsParameters, AffinePoint, EllipticCurve};
use sp1_curves::params::NumWords;
use crate::{RuntimeContext};
use crate::instruction::sp1::cast_u8_to_u32;

pub(crate) struct SyscallEdwardsAddAssign<E: EllipticCurve + EdwardsParameters> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve + EdwardsParameters> SyscallEdwardsAddAssign<E> {
    /// Create a new instance of the [`SyscallEdwardsAddAssign`].
    pub const fn new() -> Self {
        Self { _phantom: PhantomData }
    }
}

impl<E: EllipticCurve + EdwardsParameters> SyscallEdwardsAddAssign<E> {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
        let p_ptr = arg1;
        if p_ptr % 4 != 0 {
            panic!();
        }
        let q_ptr = arg2;
        if q_ptr % 4 != 0 {
            panic!();
        }

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        let p = caller.read_memory(p_ptr, num_words as u32 * 4)?;
        let q = caller.read_memory(q_ptr, num_words as u32 * 4)?;

        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        let p_affine = AffinePoint::<E>::from_words_le(p);
        let q_affine = AffinePoint::<E>::from_words_le(q);
        let result_affine = p_affine + q_affine;

        let result_words = result_affine.to_words_le();

        caller.write_memory(p_ptr, result_words.into_iter().map(|x| x.to_be_bytes()).flatten().collect::<Vec<_>>().as_slice())?;

        Ok(())
    }
}


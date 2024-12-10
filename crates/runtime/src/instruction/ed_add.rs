use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{core::Trap, Caller};
use sp1_curves::{edwards::EdwardsParameters, params::NumWords, AffinePoint, EllipticCurve};
use std::marker::PhantomData;

pub(crate) struct SyscallEdwardsAddAssign<E: EllipticCurve + EdwardsParameters> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve + EdwardsParameters> SyscallEdwardsAddAssign<E> {
    /// Create a new instance of the [`SyscallEdwardsAddAssign`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<E: EllipticCurve + EdwardsParameters> SyscallEdwardsAddAssign<E> {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        p_ptr: u32,
        q_ptr: u32,
    ) -> Result<(), Trap> {
        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        let p = caller.read_memory(p_ptr, num_words as u32 * 4)?;
        let q = caller.read_memory(q_ptr, num_words as u32 * 4)?;

        let result_vec = Self::fn_impl(p, q)?;

        caller.write_memory(p_ptr, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Result<Vec<u8>, Trap> {
        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        let p_affine = AffinePoint::<E>::from_words_le(p);
        let q_affine = AffinePoint::<E>::from_words_le(q);
        let result_affine = p_affine + q_affine;

        let result_words = result_affine.to_words_le();
        let result_vec = result_words
            .into_iter()
            .map(|x| x.to_be_bytes())
            .flatten()
            .collect::<Vec<_>>();
        Ok(result_vec)
    }
}

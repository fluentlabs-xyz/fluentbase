use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Store, TrapCode, TypedCaller, Value};
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
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (p_ptr, q_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        let mut p = vec![0; num_words * 4];
        caller.memory_read(p_ptr as usize, &mut p)?;
        let mut q = vec![0; num_words * 4];
        caller.memory_read(q_ptr as usize, &mut q)?;

        let result_vec = Self::fn_impl(&p, &q);

        caller.memory_write(p_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Vec<u8> {
        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        let p_affine = AffinePoint::<E>::from_words_le(p);
        let q_affine = AffinePoint::<E>::from_words_le(q);
        let result_affine = p_affine + q_affine;

        let result_words = result_affine.to_words_le();
        result_words
            .into_iter()
            .map(|x| x.to_be_bytes())
            .flatten()
            .collect::<Vec<_>>()
    }
}

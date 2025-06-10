use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Caller, TrapCode};
use sp1_curves::{params::NumWords, AffinePoint, EllipticCurve};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallWeierstrassDoubleAssign<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassDoubleAssign<E> {
    /// Create a new instance of the [`SyscallWeierstrassDoubleAssign`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let p_ptr: u32 = caller.stack_pop_as();

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;
        let p = caller.memory_read_vec(p_ptr as usize, num_words * 4)?;

        let result_vec = Self::fn_impl(&p);
        caller.memory_write(p_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8]) -> Vec<u8> {
        let p = cast_u8_to_u32(p).unwrap();
        let p_affine = AffinePoint::<E>::from_words_le(&p);

        let result_affine = E::ec_double(&p_affine);
        let result_words = result_affine.to_words_le();
        words_to_bytes_le_vec(result_words.as_slice())
    }
}

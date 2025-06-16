use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Caller, TrapCode};
use sp1_curves::{params::NumWords, AffinePoint, EllipticCurve};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallWeierstrassAddAssign<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassAddAssign<E> {
    /// Create a new instance of the [`SyscallWeierstrassAddAssign`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let (p_ptr, q_ptr) = caller.stack_pop2_as::<u32>();
        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        // Read p and q values from memory
        let p = caller.memory_read_vec(p_ptr as usize, num_words * 4)?;
        let q = caller.memory_read_vec(q_ptr as usize, num_words * 4)?;

        // Write the result back to memory at the p_ptr location
        let result_vec = Self::fn_impl(&p, &q);
        caller.memory_write(p_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Vec<u8> {
        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        // Convert memory to affine points
        let p_affine = AffinePoint::<E>::from_words_le(&p);
        let q_affine = AffinePoint::<E>::from_words_le(&q);

        // Perform point addition on the affine points
        let result_affine = p_affine + q_affine;

        // Convert the result back to memory format (LE words)
        let result_words = result_affine.to_words_le();

        words_to_bytes_le_vec(result_words.as_slice())
    }
}

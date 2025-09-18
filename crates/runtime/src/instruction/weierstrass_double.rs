use crate::{instruction::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Store, TrapCode, TypedCaller, Value};
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
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr: u32 = params[0].i32().unwrap() as u32;

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;
        let mut p = vec![0u8; num_words * 4];
        caller.memory_read(p_ptr as usize, &mut p)?;

        // // Wrap the fn_impl call in catch_unwind to handle panics
        // let result_vec = std::panic::catch_unwind(|| Self::fn_impl(&p)).unwrap_or_else(|_| {
        //     // If fn_impl panics, return a zero result
        //     vec![0u8; num_words * 4]
        // });

        let result_vec = Self::fn_impl(&p);
        caller.memory_write(p_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8]) -> Vec<u8> {
        let p = cast_u8_to_u32(p).unwrap();
        let p_affine = Self::safe_from_words_le(&p);

        let result_affine = E::ec_double(&p_affine);
        let result_words = result_affine.to_words_le();
        words_to_bytes_le_vec(result_words.as_slice())
    }

    /// Safely parse an affine point from words, returning identity on invalid input
    fn safe_from_words_le(words: &[u32]) -> AffinePoint<E> {
        // Check if all words are zero (identity point)
        if words.iter().all(|&w| w == 0) {
            // Create a zero point by parsing all zeros
            let zero_words = vec![0u32; words.len()];
            return AffinePoint::<E>::from_words_le(&zero_words);
        }

        // Try to parse the point, return zero point if parsing fails
        std::panic::catch_unwind(|| AffinePoint::<E>::from_words_le(words)).unwrap_or_else(|_| {
            // If parsing panics, return a zero point
            let zero_words = vec![0u32; words.len()];
            AffinePoint::<E>::from_words_le(&zero_words)
        })
    }
}

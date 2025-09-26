use crate::{syscall_handler::cast_u8_to_u32, RuntimeContext};
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{
    params::NumWords,
    weierstrass::{SwCurve, WeierstrassParameters},
    AffinePoint, BigUint,
};
use sp1_primitives::consts::words_to_bytes_le_vec;
use std::marker::PhantomData;

pub struct SyscallWeierstrassMulAssign<E: WeierstrassParameters> {
    _phantom: PhantomData<E>,
}

impl<E: WeierstrassParameters> SyscallWeierstrassMulAssign<E> {
    /// Create a new instance of the [`SyscallWeierstrassMulAssign`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (p_ptr, q_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );
        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;
        const WORD_SIZE: usize = 4;
        let param_len = num_words * WORD_SIZE;

        // Read p and q values from memory
        let mut p = vec![0u8; param_len];
        caller.memory_read(p_ptr as usize, &mut p)?;
        let mut q = vec![0u8; 8 * WORD_SIZE];
        caller.memory_read(q_ptr as usize, &mut q)?;

        // Write the result back to memory at the p_ptr location
        let result_vec = Self::fn_impl(&p, &q);
        caller.memory_write(p_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(p: &[u8], q: &[u8]) -> Vec<u8> {
        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        // Convert memory to affine points
        let p_affine = AffinePoint::<SwCurve<E>>::from_words_le(&p);
        let q_scalar = BigUint::from_slice(q);

        let result_affine = p_affine.sw_scalar_mul(&q_scalar);

        // Convert the result back to memory format (LE words)
        let result_words = result_affine.to_words_le();

        words_to_bytes_le_vec(result_words.as_slice())
    }
}

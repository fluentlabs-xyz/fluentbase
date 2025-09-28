use crate::{syscall_handler::weierstrass::weierstrass_utils::cast_u8_to_u32, RuntimeContext};
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

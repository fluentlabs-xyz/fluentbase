use std::marker::PhantomData;
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::Caller;
use rwasm::core::Trap;
use sp1_curves::{AffinePoint, CurveType, EllipticCurve};
use sp1_curves::params::NumWords;
use sp1_primitives::consts::{words_to_bytes_le, words_to_bytes_le_vec};
use crate::instruction::sp1::cast_u8_to_u32;
use crate::RuntimeContext;

pub struct SyscallWeierstrassAddAssign<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassAddAssign<E> {
    /// Create a new instance of the [`SyscallWeierstrassAddAssign`].
    pub const fn new() -> Self {
        Self { _phantom: PhantomData }
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
        let p_ptr = arg1;
        if p_ptr % 4 != 0 {
            panic!();
        }

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        let p = caller.read_memory(p_ptr, num_words as u32 * 4)?;
        let p = cast_u8_to_u32(p).unwrap();

        let p_affine = AffinePoint::<E>::from_words_le(&p);

        let result_affine = E::ec_double(&p_affine);

        let result_words = result_affine.to_words_le();

        caller.write_memory(p_ptr, &words_to_bytes_le_vec(result_words.as_slice()))?;

        Ok(())
    }
}

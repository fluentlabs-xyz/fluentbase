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
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
        let p_ptr = arg1;
        let q_ptr = arg2;

        // Ensure the pointers are 4-byte aligned
        if p_ptr % 4 != 0 || q_ptr % 4 != 0 {
            panic!("Pointer alignment error: both pointers must be 4-byte aligned.");
        }

        let num_words = <E::BaseField as NumWords>::WordsCurvePoint::USIZE;

        // Read p and q values from memory
        let p = caller.read_memory(p_ptr, num_words as u32 * 4)?;
        let q = caller.read_memory(q_ptr, num_words as u32 * 4)?;

        let p = cast_u8_to_u32(p).unwrap();
        let q = cast_u8_to_u32(q).unwrap();

        // Convert memory to affine points
        let p_affine = AffinePoint::<E>::from_words_le(&p);
        let q_affine = AffinePoint::<E>::from_words_le(&q);

        // Perform point addition on the affine points
        let result_affine = p_affine + q_affine;

        // Convert the result back to memory format (LE words)
        let result_words = result_affine.to_words_le();

        // Write the result back to memory at the p_ptr location
        caller.write_memory(p_ptr, &words_to_bytes_le_vec(result_words.as_slice()))?;


        Ok(())
    }
}

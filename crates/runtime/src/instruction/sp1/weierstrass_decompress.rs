use std::marker::PhantomData;
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::Caller;
use rwasm::core::Trap;
use sp1_curves::{AffinePoint, CurveType, EllipticCurve};
use sp1_curves::params::{NumLimbs, NumWords};
use sp1_curves::weierstrass::bls12_381::bls12381_decompress;
use sp1_curves::weierstrass::secp256k1::secp256k1_decompress;
use sp1_primitives::consts::{bytes_to_words_le_vec, words_to_bytes_le, words_to_bytes_le_vec};
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
        let slice_ptr = arg1;
        let sign_bit = arg2;

        assert!(slice_ptr % 4 == 0, "slice_ptr must be 4-byte aligned");
        assert!(sign_bit <= 1, "is_odd must be 0 or 1");


        let num_limbs = <E::BaseField as NumLimbs>::Limbs::USIZE;
        let num_words_field_element = num_limbs / 4;

        let x_bytes = caller.read_memory(slice_ptr + (num_limbs as u32), num_words_field_element as u32 * 4)?;

        let mut x_bytes_be = x_bytes.to_vec();
        x_bytes_be.reverse();

        let decompress_fn = match E::CURVE_TYPE {
            CurveType::Secp256k1 => secp256k1_decompress::<E>,
            CurveType::Bls12381 => bls12381_decompress::<E>,
            _ => panic!("Unsupported curve"),
        };

        let computed_point: AffinePoint<E> = decompress_fn(&x_bytes_be, sign_bit);

        let mut decompressed_y_bytes = computed_point.y.to_bytes_le();
        decompressed_y_bytes.resize(num_limbs, 0u8);
        let y_words = bytes_to_words_le_vec(&decompressed_y_bytes);

        caller.write_memory(slice_ptr, &words_to_bytes_le_vec(y_words.as_slice()))?;

        Ok(())
    }
}

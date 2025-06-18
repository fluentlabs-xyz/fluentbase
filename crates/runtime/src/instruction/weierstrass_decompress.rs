use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use k256::elliptic_curve::generic_array::typenum::Unsigned;
use rwasm::{Caller, TrapCode};
use sp1_curves::{
    params::NumLimbs,
    weierstrass::{bls12_381::bls12381_decompress, secp256k1::secp256k1_decompress},
    AffinePoint,
    CurveType,
    EllipticCurve,
};
use sp1_primitives::consts::{bytes_to_words_le_vec, words_to_bytes_le_vec};
use std::marker::PhantomData;

pub struct SyscallWeierstrassDecompressAssign<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassDecompressAssign<E> {
    /// Create a new instance of the [`SyscallWeierstrassDecompressAssign`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Handles the syscall for point addition on a Weierstrass curve.
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let (x_ptr, sign_bit) = caller.stack_pop2_as::<u32>();

        let num_limbs = <E::BaseField as NumLimbs>::Limbs::USIZE;
        let num_words_field_element = num_limbs / 4;

        let x_bytes = caller.memory_read_vec(
            (x_ptr + (num_limbs as u32)) as usize,
            num_words_field_element * 4,
        )?;

        let result_vec = Self::fn_impl(&x_bytes, sign_bit).map_err(|err| {
            caller.context_mut().execution_result.exit_code = err.into_i32();
            TrapCode::ExecutionHalted
        })?;
        caller.memory_write(x_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(x_bytes: &[u8], sign_bit: u32) -> Result<Vec<u8>, ExitCode> {
        if sign_bit > 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let num_limbs = <E::BaseField as NumLimbs>::Limbs::USIZE;

        let mut x_bytes_be = x_bytes.to_vec();
        x_bytes_be.reverse();

        let decompress_fn = match E::CURVE_TYPE {
            CurveType::Secp256k1 => secp256k1_decompress::<E>,
            CurveType::Bls12381 => bls12381_decompress::<E>,
            _ => panic!("unsupported curve"),
        };

        let computed_point: AffinePoint<E> = decompress_fn(&x_bytes_be, sign_bit);

        let mut decompressed_y_bytes = computed_point.y.to_bytes_le();
        decompressed_y_bytes.resize(num_limbs, 0u8);
        let y_words = bytes_to_words_le_vec(&decompressed_y_bytes);

        let result_vec = words_to_bytes_le_vec(&y_words);
        Ok(result_vec)
    }
}

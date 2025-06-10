use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode};
use sp1_curves::{
    curve25519_dalek::CompressedEdwardsY,
    edwards::{ed25519::decompress, EdwardsParameters, WORDS_FIELD_ELEMENT},
    COMPRESSED_POINT_BYTES,
};
use std::marker::PhantomData;

pub(crate) struct SyscallEdwardsDecompress<E: EdwardsParameters> {
    _phantom: PhantomData<E>,
}

impl<E: EdwardsParameters> SyscallEdwardsDecompress<E> {
    /// Create a new instance of the [`SyscallEdwardsDecompress`].
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    #[allow(clippy::many_single_char_names)]
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), TrapCode> {
        let (slice_ptr, sign) = caller.stack_pop2();
        // Read the Y bytes from memory
        let y_bytes = caller
            .memory_read_fixed::<{ WORDS_FIELD_ELEMENT * 4 }>(
                slice_ptr.as_usize() + COMPRESSED_POINT_BYTES,
            )?
            .try_into()
            .unwrap();
        let result_vec = Self::fn_impl(y_bytes, sign.as_u32()).map_err(|err| {
            caller.context_mut().execution_result.exit_code = err.into();
            TrapCode::ExecutionHalted
        })?;
        // Write the decompressed X back to memory
        caller.memory_write(slice_ptr.as_usize(), &result_vec)?;
        Ok(())
    }

    pub fn fn_impl(mut y_bytes: [u8; 32], sign: u32) -> Result<Vec<u8>, ExitCode> {
        if sign > 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        // Copy bytes into another array so we can modify the last byte and make CompressedEdwardsY
        y_bytes[COMPRESSED_POINT_BYTES - 1] &= 0b0111_1111;
        y_bytes[COMPRESSED_POINT_BYTES - 1] |= (sign as u8) << 7;

        // Compute actual decompressed X
        let compressed_y = CompressedEdwardsY(y_bytes);
        let decompressed = decompress(&compressed_y).ok_or(ExitCode::MalformedBuiltinParams)?;

        // Convert the decompressed X to bytes and then words
        let mut decompressed_x_bytes = decompressed.x.to_bytes_le();
        decompressed_x_bytes.resize(32, 0u8); // Ensure it has the correct size

        Ok(decompressed_x_bytes)
    }
}

use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{core::Trap, Caller};
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
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        slice_ptr: u32,
        sign: u32,
    ) -> Result<(), Trap> {
        // Read the Y bytes from memory
        let y_bytes = caller
            .read_memory(
                slice_ptr + COMPRESSED_POINT_BYTES as u32,
                WORDS_FIELD_ELEMENT as u32 * 4,
            )?
            .try_into()
            .unwrap();
        let result_vec = Self::fn_impl(y_bytes, sign)?;
        // Write the decompressed X back to memory
        caller.write_memory(slice_ptr, &result_vec)?;
        Ok(())
    }

    pub fn fn_impl(mut y_bytes: [u8; 32], sign: u32) -> Result<Vec<u8>, Trap> {
        if sign > 1 {
            return Err(ExitCode::BadBuiltinParams.into_trap());
        }

        // Copy bytes into another array so we can modify the last byte and make CompressedEdwardsY
        y_bytes[COMPRESSED_POINT_BYTES - 1] &= 0b0111_1111;
        y_bytes[COMPRESSED_POINT_BYTES - 1] |= (sign as u8) << 7;

        // Compute actual decompressed X
        let compressed_y = CompressedEdwardsY(y_bytes);
        let decompressed = decompress(&compressed_y);

        // Convert the decompressed X to bytes and then words
        let mut decompressed_x_bytes = decompressed.x.to_bytes_le();
        decompressed_x_bytes.resize(32, 0u8); // Ensure it has the correct size

        Ok(decompressed_x_bytes)
    }
}

use std::marker::PhantomData;
use rwasm::Caller;
use rwasm::core::Trap;

use sp1_curves::{
    curve25519_dalek::CompressedEdwardsY,
    edwards::{ed25519::decompress, EdwardsParameters, WORDS_FIELD_ELEMENT},
    COMPRESSED_POINT_BYTES,
};
use sp1_primitives::consts::{bytes_to_words_le, words_to_bytes_le};

use crate::{RuntimeContext};

pub(crate) struct SyscallEdwardsDecompress<E: EdwardsParameters> {
    _phantom: PhantomData<E>,
}

impl<E: EdwardsParameters> SyscallEdwardsDecompress<E> {
    /// Create a new instance of the [`SyscallEdwardsDecompress`].
    pub const fn new() -> Self {
        Self { _phantom: PhantomData }
    }

    #[allow(clippy::many_single_char_names)]
    pub fn execute(mut caller: Caller<'_, RuntimeContext>, slice_ptr: u32, sign: u32) -> Result<(), Trap> {
        assert!(slice_ptr % 4 == 0, "Pointer must be 4-byte aligned.");
        assert!(sign <= 1, "Sign bit must be 0 or 1.");

        // Read the Y bytes from memory
        let y_bytes = caller.read_memory(slice_ptr + COMPRESSED_POINT_BYTES as u32, WORDS_FIELD_ELEMENT as u32 * 4)?
            .try_into()
            .unwrap();

        // Copy bytes into another array so we can modify the last byte and make CompressedEdwardsY
        let mut compressed_edwards_y: [u8; COMPRESSED_POINT_BYTES] = y_bytes;
        compressed_edwards_y[COMPRESSED_POINT_BYTES - 1] &= 0b0111_1111;
        compressed_edwards_y[COMPRESSED_POINT_BYTES - 1] |= (sign as u8) << 7;

        // Compute actual decompressed X
        let compressed_y = CompressedEdwardsY(compressed_edwards_y);
        let decompressed = decompress(&compressed_y);

        // Convert the decompressed X to bytes and then words
        let mut decompressed_x_bytes = decompressed.x.to_bytes_le();
        decompressed_x_bytes.resize(32, 0u8); // Ensure it has the correct size


        // Write the decompressed X back to memory
        caller.write_memory(slice_ptr, &decompressed_x_bytes)?;

        Ok(())
    }
}

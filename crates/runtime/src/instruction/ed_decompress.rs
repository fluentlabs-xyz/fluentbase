use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};
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
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (slice_ptr, sign) = (
            params[0].i32().unwrap() as usize,
            params[1].i32().unwrap() as u32,
        );
        // Read the Y bytes from memory
        let mut y_bytes = [0u8; WORDS_FIELD_ELEMENT * 4];
        caller.memory_read(slice_ptr + COMPRESSED_POINT_BYTES, &mut y_bytes)?;
        let result_vec = Self::fn_impl(y_bytes, sign).map_err(|err| {
            caller.context_mut(|ctx| ctx.execution_result.exit_code = err.into());
            TrapCode::ExecutionHalted
        })?;
        // Write the decompressed X back to memory
        caller.memory_write(slice_ptr, &result_vec)?;
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

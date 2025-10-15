/// This code should be 100% compatible with SP1's version:
/// - sp1/crates/core/executor/src/syscalls/precompiles/edwards/decompress.rs
///
/// P.S: Instead of constraint check for `sign<=1` we emit exit code (`MalformedBuiltinParams`),
///  that must be represented inside rWasm zkVM.
use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use fluentbase_types::{ExitCode, ED25519_POINT_COMPRESSED_SIZE, ED25519_POINT_DECOMPRESSED_SIZE};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{curve25519_dalek::CompressedEdwardsY, edwards::ed25519::decompress};


pub fn syscall_ed25519_decompress_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let slice_ptr = params[0].i32().unwrap() as usize;
    let sign = params[1].i32().unwrap() as u32;
    let mut compressed_edwards_y = [0u8; ED25519_POINT_COMPRESSED_SIZE];
    ctx.memory_read(
        slice_ptr + ED25519_POINT_COMPRESSED_SIZE,
        &mut compressed_edwards_y,
    )?;
    let decompressed_x_bytes = syscall_ed25519_decompress_impl(compressed_edwards_y, sign)
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;
    ctx.memory_write(slice_ptr, &decompressed_x_bytes)?;
    Ok(())
}

pub fn syscall_ed25519_decompress_impl(
    mut compressed_edwards_y: [u8; ED25519_POINT_COMPRESSED_SIZE],
    sign: u32,
) -> Result<[u8; ED25519_POINT_DECOMPRESSED_SIZE], ExitCode> {
    // TODO(dmitry123): If we don't have this check, then constraint violation might happen inside SP1
    if sign > 1 {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let mut result = [0u8; ED25519_POINT_DECOMPRESSED_SIZE];
    result[32..64].copy_from_slice(&compressed_edwards_y);
    // Re-insert sign bit into last bit of Y for CompressedEdwardsY format
    compressed_edwards_y[ED25519_POINT_COMPRESSED_SIZE - 1] &= 0b0111_1111;
    compressed_edwards_y[ED25519_POINT_COMPRESSED_SIZE - 1] |= (sign as u8) << 7;
    // Compute actual decompressed X
    let compressed_y = CompressedEdwardsY(compressed_edwards_y);
    let decompressed = match decompress(&compressed_y) {
        Some(decompressed) => decompressed,
        None => return Err(ExitCode::MalformedBuiltinParams),
    };
    let mut decompressed_x_bytes = decompressed.x.to_bytes_le();
    decompressed_x_bytes.resize(32, 0u8);
    result[0..32].copy_from_slice(decompressed_x_bytes.as_slice());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_types::hex;

    /// SP1 tests are taken from: sp1/crates/test-artifacts/programs/ed-decompress/src/main.rs
    #[test]
    fn test_ed25519_decompress_sp1() {
        let mut pub_bytes: [u8; 32] =
            hex!("ec172b93ad5e563bf4932c70e1245034c35467ef2efd4d64ebf819683467e2bf");
        let sign = pub_bytes[31] >> 7;
        pub_bytes[31] &= 0b0111_1111;
        let decompressed = syscall_ed25519_decompress_impl(pub_bytes, sign as u32).unwrap();
        let expected: [u8; 64] = [
            47, 252, 114, 91, 153, 234, 110, 201, 201, 153, 152, 14, 68, 231, 90, 221, 137, 110,
            250, 67, 10, 64, 37, 70, 163, 101, 111, 223, 185, 1, 180, 88, 236, 23, 43, 147, 173,
            94, 86, 59, 244, 147, 44, 112, 225, 36, 80, 52, 195, 84, 103, 239, 46, 253, 77, 100,
            235, 248, 25, 104, 52, 103, 226, 63,
        ];
        assert_eq!(hex::encode(decompressed), hex::encode(expected));
    }

    /// Verifies that invalid Y coordinates return an error instead of panicking (DoS prevention).
    ///
    /// Uses Y=2 which is not on the Ed25519 curve (u/v is not a quadratic residue).
    /// This ensures `decompress()` returns `None` and the syscall handles it gracefully
    /// with `ExitCode::MalformedBuiltinParams` rather than crashing the VM.
    #[test]
    fn test_ed25519_decompress_invalid_input_returns_error() {
        let sign = 0;
        // Y=2: mathematically proven to not satisfy Ed25519 curve equation
        let invalid_y: [u8; 32] = hex!("0200000000000000000000000000000000000000000000000000000000000000");

        let result = syscall_ed25519_decompress_impl(invalid_y, sign);

        assert!(result.is_err(), "Expected error for invalid point, got: {:?}", result);
        assert_eq!(result.unwrap_err(), ExitCode::MalformedBuiltinParams);
    }
}

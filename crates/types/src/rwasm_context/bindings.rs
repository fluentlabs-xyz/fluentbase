/// Check [crate::NativeAPI] for docs.
#[link(wasm_import_module = "fluentbase_v1preview")]
extern "C" {
    // input/output & state control (0x00)
    pub fn _exit(code: i32) -> !;
    pub fn _state() -> u32;
    // TODO(dmitry123): "rename to `_read_input`"
    pub fn _read(target: *mut u8, offset: u32, length: u32);
    pub fn _input_size() -> u32;
    // TODO(dmitry123): "rename to `_write_output`"
    pub fn _write(offset: *const u8, length: u32);
    pub fn _output_size() -> u32;
    pub fn _read_output(target: *mut u8, offset: u32, length: u32);
    pub fn _exec(
        hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        fuel16_ptr: *mut [i64; 2],
        state: u32,
    ) -> i32;
    pub fn _resume(
        call_id: u32,
        return_data_ptr: *const u8,
        return_data_len: u32,
        exit_code: i32,
        fuel16_ptr: *mut [i64; 2],
    ) -> i32;
    pub fn _forward_output(offset: u32, len: u32);
    pub fn _fuel() -> u64;
    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
    pub fn _charge_fuel(fuel_consumed: u64);
    pub fn _enter_unconstrained();
    pub fn _exit_unconstrained();
    pub fn _write_fd(fd: u32, slice_ptr: *const u8, slice_len: u32);

    // hashing functions (0x01)
    #[deprecated(note = "will be removed in fluentbase_v1 schema version")]
    pub fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _keccak256_permute(state_ptr: *mut [u64; 25]);
    #[deprecated(note = "will be removed in fluentbase_v1 schema version")]
    pub fn _poseidon(
        parameters: u32,
        endianness: u32,
        data_offset: *const u8,
        data_len: u32,
        output32_offset: *mut u8,
    ) -> u32;
    pub fn _sha256_extend(w: *mut [u32; 64]);
    pub fn _sha256_compress(state: *mut [u32; 8], w: *const [u32; 64]);
    #[deprecated(note = "will be removed in fluentbase_v1 schema version")]
    pub fn _sha256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    #[deprecated(note = "will be removed in fluentbase_v1 schema version")]
    pub fn _blake3(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);

    // ed25519 (0x02)
    pub fn _ed25519_decompress(slice_ptr: *mut u8, sign: u32);
    pub fn _ed25519_add(p_ptr: *mut u8, q_ptr: *const u8);

    // fp1/fp2 tower field (0x03)
    pub fn _tower_fp1_bn254_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bn254_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bn254_mul(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_mul(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bn254_add(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );
    pub fn _tower_fp2_bn254_sub(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );
    pub fn _tower_fp2_bn254_mul(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );
    pub fn _tower_fp2_bls12381_add(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );
    pub fn _tower_fp2_bls12381_sub(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );
    pub fn _tower_fp2_bls12381_mul(
        x_c0_ptr: *mut u8,
        x_c1_ptr: *mut u8,
        y_c0_ptr: *const u8,
        y_c1_ptr: *const u8,
    );

    // secp256k1 (0x04)
    pub fn _secp256k1_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _secp256k1_decompress(x_ptr: *mut u8, sign: u32);
    pub fn _secp256k1_double(p_ptr: *mut u8);

    // secp256r1 (0x05)
    pub fn _secp256r1_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _secp256r1_decompress(x_ptr: *mut u8, sign: u32);
    pub fn _secp256r1_double(p_ptr: *mut u8);

    // bls12381 (0x06)
    pub fn _bls12381_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _bls12381_decompress(x_ptr: *mut u8, sign: u32);
    pub fn _bls12381_double(p_ptr: *mut u8);

    // bn254 (0x07)
    pub fn _bn254_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _bn254_double(p_ptr: *mut u8);

    // uint256 (0x08)
    pub fn _uint256_mul_mod(x32_ptr: *mut u8, y32_ptr: *const u8, m32_ptr: *const u8);
    pub fn _uint256_x2048_mul(a_ptr: *const u8, b_ptr: *const u8, lo_ptr: *mut u8, hi_ptr: *mut u8);
}

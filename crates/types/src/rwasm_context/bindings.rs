/// Check [crate::NativeAPI] for docs.
#[link(wasm_import_module = "fluentbase_v1preview")]
extern "C" {
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
    pub fn _charge_fuel_manually(fuel_consumed: u64, fuel_refunded: i64) -> u64;
    pub fn _fuel() -> u64;
    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
    pub fn _charge_fuel(fuel_consumed: u64);

    // TODO(dmitry123): Delete me
    pub fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _keccak256_permute(state_ptr: *mut [u64; 25]);
    // TODO(dmitry123): Delete me
    pub fn _poseidon(
        parameters: u32,
        endianness: u32,
        data_offset: *const u8,
        data_len: u32,
        output32_offset: *mut u8,
    ) -> u32;
    pub fn _sha256_extend(w_ptr: *mut u8);
    pub fn _sha256_compress(w_ptr: *mut u8, h_ptr: *const u8);
    // TODO(dmitry123): Delete me
    pub fn _sha256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    // TODO(dmitry123): Delete me
    pub fn _blake3(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);

    pub fn _ed25519_decompress(slice_ptr: *mut u8, sign: u32);
    pub fn _ed25519_add(p_ptr: *mut u8, q_ptr: *const u8);

    pub fn _tower_fp1_bn254_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bn254_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bn254_mul(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp1_bls12381_mul(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bn254_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bn254_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bn254_mul(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bls12381_add(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bls12381_sub(x_ptr: *mut u8, y_ptr: *const u8);
    pub fn _tower_fp2_bls12381_mul(x_ptr: *mut u8, y_ptr: *const u8);

    pub fn _secp256k1_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _secp256k1_decompress(x_ptr: *mut u8, sign: u32);
    pub fn _secp256k1_double(p_ptr: *mut u8);

    pub fn _bls12381_g1_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _bls12381_g1_msm(pairs_ptr: *const u8, pairs_count: u32, out_ptr: *mut u8);
    pub fn _bls12381_g2_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _bls12381_g2_msm(pairs_ptr: *const u8, pairs_count: u32, out_ptr: *mut u8);
    // TODO(dmitry123): Delete me
    pub fn _bls12381_pairing(pairs_ptr: *const u8, pairs_count: u32, out_ptr: *mut u8);
    pub fn _bls12381_map_g1(p_ptr: *const u8, out_ptr: *mut u8);
    pub fn _bls12381_map_g2(p_ptr: *const u8, out_ptr: *mut u8);

    pub fn _bn254_add(arg1: u32, arg2: u32);
    pub fn _bn254_double(p_ptr: u32);
    pub fn _bn254_mul(arg1: u32, arg2: u32);
    pub fn _bn254_multi_pairing(elements_ptr: *const u8, elements_count: u32, ret_ptr: *mut u8);
    pub fn _bn254_g1_compress(point_ptr: *const u8, ret_ptr: *mut u8) -> u32;
    pub fn _bn254_g1_decompress(point_ptr: *const u8, ret_ptr: *mut u8) -> u32;
    pub fn _bn254_g2_compress(point_ptr: *const u8, ret_ptr: *mut u8) -> u32;
    pub fn _bn254_g2_decompress(point_ptr: *const u8, ret_ptr: *mut u8) -> u32;

    pub fn _uint256_mul_mod(x32_ptr: *mut u8, y32_ptr: *const u8, m32_ptr: *const u8);
    pub fn _uint256_x2048_mul(
        a32_ptr: *const u8,
        b32_ptr: *const u8,
        lo32_ptr: *mut u8,
        hi32_ptr: *mut u8,
    );
}

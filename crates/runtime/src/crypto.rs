use crate::{syscall_handler::*, RuntimeContextWrapper};
use fluentbase_types::{
    CryptoAPI, UnwrapExitCode, BLS12381_FP_SIZE, BLS12381_G1_COMPRESSED_SIZE,
    BLS12381_G1_RAW_AFFINE_SIZE, BN254_FP_SIZE, BN254_G1_RAW_AFFINE_SIZE,
    ED25519_POINT_COMPRESSED_SIZE, ED25519_POINT_DECOMPRESSED_SIZE, SECP256K1_G1_COMPRESSED_SIZE,
    SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE, SECP256R1_G1_RAW_AFFINE_SIZE,
};

#[rustfmt::skip]
impl CryptoAPI for RuntimeContextWrapper {
    #[inline(always)]
    fn keccak256_permute(state: &mut [u64; 25]) {
        syscall_hashing_keccak256_permute_impl(state);
    }
    #[inline(always)]
    fn sha256_extend(w: &mut [u32; 64]) {
        syscall_hashing_sha256_extend_impl(w);
    }
    #[inline(always)]
    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]) {
        syscall_hashing_sha256_compress_impl(state, w);
    }

    #[inline(always)]
    fn ed25519_decompress(y: [u8; ED25519_POINT_COMPRESSED_SIZE], sign: u32) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE] {
        syscall_ed25519_decompress_impl(y, sign).unwrap_exit_code()
    }
    #[inline(always)]
    fn ed25519_add(p: [u8; ED25519_POINT_DECOMPRESSED_SIZE], q: [u8; ED25519_POINT_DECOMPRESSED_SIZE]) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE] {
        syscall_edwards_add_impl(p, q).unwrap_exit_code()
    }

    #[inline(always)]
    fn tower_fp1_bn254_add(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        syscall_tower_fp1_bn254_add_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp1_bn254_sub(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        syscall_tower_fp1_bn254_sub_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp1_bn254_mul(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        syscall_tower_fp1_bn254_mul_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp1_bls12381_add(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        syscall_tower_fp1_bls12381_add_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp1_bls12381_sub(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        syscall_tower_fp1_bls12381_sub_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp1_bls12381_mul(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        syscall_tower_fp1_bls12381_mul_impl(x, y).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bn254_add(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        syscall_tower_fp2_bn254_add_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bn254_sub(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        syscall_tower_fp2_bn254_sub_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bn254_mul(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        syscall_tower_fp2_bn254_mul_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bls12381_add(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        syscall_tower_fp2_bls12381_add_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bls12381_sub(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        syscall_tower_fp2_bls12381_sub_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }
    #[inline(always)]
    fn tower_fp2_bls12381_mul(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        syscall_tower_fp2_bls12381_mul_impl(a_c0, a_c1, b_c0, b_c1).unwrap_exit_code()
    }

    #[inline(always)]
    fn secp256k1_add(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        syscall_secp256k1_add_impl(p, q).unwrap_exit_code()
    }
    #[inline(always)]
    fn secp256k1_decompress(mut x: [u8; SECP256K1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        x.reverse();
        let mut result = syscall_secp256k1_decompress_impl(x, sign).unwrap_exit_code();
        result.reverse();
        result
    }
    #[inline(always)]
    fn secp256k1_double(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        syscall_secp256k1_double_impl(p).unwrap_exit_code()
    }

    #[inline(always)]
    fn secp256r1_add(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        syscall_secp256r1_add_impl(p, q).unwrap_exit_code()
    }
    #[inline(always)]
    fn secp256r1_decompress(mut x: [u8; SECP256R1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        x.reverse();
        let mut result = syscall_secp256r1_decompress_impl(x, sign).unwrap_exit_code();
        result.reverse();
        result
    }
    #[inline(always)]
    fn secp256r1_double(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        syscall_secp256r1_double_impl(p).unwrap_exit_code()
    }

    #[inline(always)]
    fn bls12381_add(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE], q: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        syscall_bls12381_add_impl(p, q).unwrap_exit_code()
    }
    #[inline(always)]
    fn bls12381_decompress(mut x: [u8; BLS12381_G1_COMPRESSED_SIZE], sign: u32) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        x.reverse();
        let mut result = syscall_bls12381_decompress_impl(x, sign).unwrap_exit_code();
        result.reverse();
        result
    }
    #[inline(always)]
    fn bls12381_double(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        syscall_bls12381_double_impl(p).unwrap_exit_code()
    }

    #[inline(always)]
    fn bn254_add(p: [u8; BN254_G1_RAW_AFFINE_SIZE], q: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE] {
        syscall_bn254_add_impl(p, q).unwrap_exit_code()
    }
    #[inline(always)]
    fn bn254_double(p: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE] {
        syscall_bn254_double_impl(p).unwrap_exit_code()
    }

    #[inline(always)]
    fn uint256_mul_mod(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        syscall_uint256_mul_mod_impl(x, y, m)
    }
    #[inline(always)]
    fn uint256_x2048_mul(a: &[u8; 32], b: &[u8; 256]) -> ([u8; 256], [u8; 32]) {
        syscall_uint256_x2048_mul_impl(a, b)
    }
}

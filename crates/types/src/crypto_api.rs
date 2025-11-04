use crate::{
    BLS12381_FP_SIZE, BLS12381_G1_COMPRESSED_SIZE, BLS12381_G1_RAW_AFFINE_SIZE, BN254_FP_SIZE,
    BN254_G1_RAW_AFFINE_SIZE, ED25519_POINT_COMPRESSED_SIZE, ED25519_POINT_DECOMPRESSED_SIZE,
    SECP256K1_G1_COMPRESSED_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE,
    SECP256R1_G1_RAW_AFFINE_SIZE,
};

/// A low-level API that provides access to crypto-related functions, including:
/// - hashing (keccak256, sha256)
/// - ed25519
/// - tower field
/// - bls12381
/// - bn254
/// - secp256k1
/// - secp256r1

#[rustfmt::skip]
pub trait CryptoAPI {
    fn keccak256_permute(state: &mut [u64; 25]);
    fn sha256_extend(w: &mut [u32; 64]);
    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]);

    fn ed25519_decompress(y: [u8; ED25519_POINT_COMPRESSED_SIZE], sign: u32) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE];
    fn ed25519_add(p: [u8; ED25519_POINT_DECOMPRESSED_SIZE], q: [u8; ED25519_POINT_DECOMPRESSED_SIZE]) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE];

    fn tower_fp1_bn254_add(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];
    fn tower_fp1_bn254_sub(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];
    fn tower_fp1_bn254_mul(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];
    fn tower_fp1_bls12381_add(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];
    fn tower_fp1_bls12381_sub(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];
    fn tower_fp1_bls12381_mul(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];
    fn tower_fp2_bn254_add(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);
    fn tower_fp2_bn254_sub(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);
    fn tower_fp2_bn254_mul(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);
    fn tower_fp2_bls12381_add(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);
    fn tower_fp2_bls12381_sub(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);
    fn tower_fp2_bls12381_mul(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);

    fn secp256k1_add(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];
    fn secp256k1_decompress(x: [u8; SECP256K1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];
    fn secp256k1_double(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];

    fn secp256r1_add(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];
    fn secp256r1_decompress(x: [u8; SECP256R1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];
    fn secp256r1_double(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];

    fn bls12381_add(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE], q: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];
    fn bls12381_decompress(x: [u8; BLS12381_G1_COMPRESSED_SIZE], sign: u32) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];
    fn bls12381_double(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];

    fn bn254_add(p: [u8; BN254_G1_RAW_AFFINE_SIZE], q: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE];
    fn bn254_double(p: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE];

    fn uint256_mul_mod(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32];
    fn uint256_x2048_mul(a: &[u8; 32], b: &[u8; 256]) -> ([u8; 256], [u8; 32]);
}

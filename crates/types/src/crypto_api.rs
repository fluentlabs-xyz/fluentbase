use crate::{
    ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, EDWARDS_COMPRESSED_SIZE,
    EDWARDS_DECOMPRESSED_SIZE, SECP256K1_POINT_DECOMPRESSED_SIZE, TOWER_FP_BLS12381_SIZE,
    TOWER_FP_BN256_SIZE,
};

/// A low-level API that provides access to crypto-related functions, including:
/// - hashing (keccak256, sha256)
/// - ed25519
/// - tower field
/// - bls12381
/// - bn254
/// - secp256k1
/// - secp256r1

pub trait CryptoAPI {
    fn keccak256_permute(state: &mut [u64; 25]);
    fn sha256_extend(w: &mut [u32; 64]);
    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]);

    fn ed25519_decompress(
        y: [u8; EDWARDS_COMPRESSED_SIZE],
        sign: u32,
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE];
    fn ed25519_add(
        p: [u8; EDWARDS_DECOMPRESSED_SIZE],
        q: [u8; EDWARDS_DECOMPRESSED_SIZE],
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE];

    fn tower_fp1_bn254_add(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bn254_sub(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bn254_mul(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bls12381_add(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp1_bls12381_sub(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp1_bls12381_mul(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bn254_add(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bn254_sub(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bn254_mul(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bls12381_add(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bls12381_sub(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bls12381_mul(
        x: [u8; TOWER_FP_BLS12381_SIZE],
        y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE];

    fn _secp256k1_add(
        p: &mut [u8; SECP256K1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; SECP256K1_POINT_DECOMPRESSED_SIZE],
    );
    // fn _secp256k1_decompress(x_ptr: *mut u8, sign: u32);
    fn _secp256k1_double(p: &mut [u8; SECP256K1_POINT_DECOMPRESSED_SIZE]);

    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]);
    fn bls12_381_g1_msm(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]);
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]);
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]);
    #[deprecated(note = "will be removed")]
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]);
    fn bls12_381_map_g1(p: &[u8; 64], out: &mut [u8; 96]);
    fn bls12_381_map_g2(p: &[u8; 128], out: &mut [u8; 192]);

    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) -> [u8; 64];
    fn bn254_double(p: &mut [u8; 64]);
    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) -> Result<[u8; 64], ExitCode>;
    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> Result<[u8; 32], ExitCode>;
    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode>;
    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode>;
    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode>;
    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode>;

    fn uint256_mul_mod(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32];
    fn uint256_x2048_mul(a: &[u8; 32], b: &[u8; 256]) -> ([u8; 256], [u8; 32]);
}

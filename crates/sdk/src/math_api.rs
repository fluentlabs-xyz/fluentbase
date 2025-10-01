use crate::{
    ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, EDWARDS_COMPRESSED_SIZE,
    EDWARDS_DECOMPRESSED_SIZE, TOWER_FP_BLS12381_SIZE, TOWER_FP_BN256_SIZE,
};

#[rustfmt::skip]
pub trait MathAPI {
    fn ed25519_decompress(y: [u8; EDWARDS_COMPRESSED_SIZE], sign: u32) -> [u8; EDWARDS_DECOMPRESSED_SIZE];
    fn ed25519_add(p: [u8; EDWARDS_DECOMPRESSED_SIZE], q: [u8; EDWARDS_DECOMPRESSED_SIZE]) -> [u8; EDWARDS_DECOMPRESSED_SIZE];

    fn tower_fp1_bn254_add(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bn254_sub(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bn254_mul(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp1_bls12381_add(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp1_bls12381_sub(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp1_bls12381_mul(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bn254_add(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bn254_sub(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bn254_mul(_x: [u8; TOWER_FP_BN256_SIZE], _y: [u8; TOWER_FP_BN256_SIZE]) -> [u8; TOWER_FP_BN256_SIZE];
    fn tower_fp2_bls12381_add(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bls12381_sub(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];
    fn tower_fp2_bls12381_mul(_x: [u8; TOWER_FP_BLS12381_SIZE], _y: [u8; TOWER_FP_BLS12381_SIZE]) -> [u8; TOWER_FP_BLS12381_SIZE];

    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]);
    fn bls12_381_g1_msm(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]);
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]);
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]);
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]);
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 96]);
    fn bls12_381_map_fp2_to_g2(p: &[u8; 128], out: &mut [u8; 192]);
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
    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]);
    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]);
}

use crate::{
    BLS12381_FP_SIZE, BLS12381_G1_COMPRESSED_SIZE, BLS12381_G1_RAW_AFFINE_SIZE, BN254_FP_SIZE,
    BN254_G1_RAW_AFFINE_SIZE, ED25519_POINT_COMPRESSED_SIZE, ED25519_POINT_DECOMPRESSED_SIZE,
    SECP256K1_G1_COMPRESSED_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE,
    SECP256R1_G1_RAW_AFFINE_SIZE,
};

/// A low-level API for cryptographic primitives used across the runtime.
#[rustfmt::skip]
pub trait CryptoAPI {
    /// In-place `Keccak-f[1600]` permutation over 25 lanes of 64-bit.
    ///
    /// Input/Output: `state` is the 5x5x64-bit state flattened to 25 u64 words (little-endian lanes).
    fn keccak256_permute(state: &mut [u64; 25]);

    /// Expand/prepare the SHA-256 message schedule in-place.
    ///
    /// Input/Output: `w` holds 64 32-bit words; indices 16..63 are filled using the σ0/σ1 recurrences.
    fn sha256_extend(w: &mut [u32; 64]);

    /// One SHA-256 compression round.
    ///
    /// Inputs: `state` is the current 8-word state; `w` is the 64-word message schedule.
    /// Output: `state` is updated in-place with the standard SHA-256 round function.
    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]);

    /// Decompress an Ed25519 point from compressed y and a sign bit.
    ///
    /// Inputs: `y` is 32-byte compressed y-coordinate; `sign` selects the x parity.
    /// Output: 64-byte raw affine point encoded as x||y (little-endian per coordinate).
    fn ed25519_decompress(y: [u8; ED25519_POINT_COMPRESSED_SIZE], sign: u32) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE];

    /// Add two Ed25519 points in raw affine form.
    ///
    /// Inputs: `p`, `q` are 64-byte x||y encodings.
    /// Output: 64-byte x||y result in the prime-order subgroup.
    fn ed25519_add(p: [u8; ED25519_POINT_DECOMPRESSED_SIZE], q: [u8; ED25519_POINT_DECOMPRESSED_SIZE]) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE];

    /// BN254 base field (Fp) addition: (x + y) mod p.
    /// Inputs/Output: 32-byte little-endian field elements.
    fn tower_fp1_bn254_add(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];

    /// BN254 base field (Fp) subtraction: (x - y) mod p.
    /// Inputs/Output: 32-byte little-endian field elements.
    fn tower_fp1_bn254_sub(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];

    /// BN254 base field (Fp) multiplication: (x * y) mod p.
    /// Inputs/Output: 32-byte little-endian field elements.
    fn tower_fp1_bn254_mul(x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE];

    /// BLS12-381 base field (Fp) addition: (x + y) mod p.
    /// Inputs/Output: 48-byte little-endian field elements.
    fn tower_fp1_bls12381_add(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];

    /// BLS12-381 base field (Fp) subtraction: (x - y) mod p.
    /// Inputs/Output: 48-byte little-endian field elements.
    fn tower_fp1_bls12381_sub(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];

    /// BLS12-381 base field (Fp) multiplication: (x * y) mod p.
    /// Inputs/Output: 48-byte little-endian field elements.
    fn tower_fp1_bls12381_mul(x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE];

    /// BN254 quadratic extension field (Fp2) addition.
    ///
    /// Each Fp2 element is (c0, c1) over BN254 Fp, each limb 32-byte little-endian.
    /// Returns (sum_c0, sum_c1).
    fn tower_fp2_bn254_add(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);

    /// BN254 quadratic extension field (Fp2) subtraction.
    /// Returns (diff_c0, diff_c1).
    fn tower_fp2_bn254_sub(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);

    /// BN254 quadratic extension field (Fp2) multiplication.
    /// Returns (prod_c0, prod_c1) reduced modulo p.
    fn tower_fp2_bn254_mul(a_c0: [u8; BN254_FP_SIZE], a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]);

    /// BLS12-381 quadratic extension field (Fp2) addition.
    /// Each limb is 48-byte little-endian; returns (sum_c0, sum_c1).
    fn tower_fp2_bls12381_add(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);

    /// BLS12-381 quadratic extension field (Fp2) subtraction.
    /// Returns (diff_c0, diff_c1).
    fn tower_fp2_bls12381_sub(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);

    /// BLS12-381 quadratic extension field (Fp2) multiplication.
    /// Returns (prod_c0, prod_c1) reduced modulo p.
    fn tower_fp2_bls12381_mul(a_c0: [u8; BLS12381_FP_SIZE], a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]);

    /// Add two secp256k1 G1 points (affine x||y, 64 bytes total).
    /// Returns the affine sum encoded as x||y (little-endian coordinates).
    fn secp256k1_add(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];

    /// Decompress a secp256k1 point from x and sign bit.
    /// Inputs: `x` is 32-byte x (big-endian); `sign` selects the y root.
    /// Output: 64-byte x||y (big-endian per coordinate).
    fn secp256k1_decompress(x: [u8; SECP256K1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];

    /// Point doubling on secp256k1.
    /// Input: affine x||y; Output: affine x||y.
    fn secp256k1_double(p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE];

    /// Add two secp256r1 (P-256) G1 points (affine x||y, 64 bytes total).
    /// Returns the affine sum encoded as x||y (little-endian coordinates).
    fn secp256r1_add(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];

    /// Decompress a secp256r1 point from x and sign bit.
    /// Inputs: `x` is 32-byte x (big-endian); `sign` selects the y root.
    /// Output: 64-byte x||y (big-endian per coordinate).
    fn secp256r1_decompress(x: [u8; SECP256R1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];

    /// Point doubling on secp256r1.
    /// Input: affine x||y; Output: affine x||y.
    fn secp256r1_double(p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE];

    /// Add two BLS12-381 G1 points (affine x||y, 96 bytes total).
    /// Returns the affine sum encoded as x||y (little-endian coordinates).
    fn bls12381_add(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE], q: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];

    /// Decompress a BLS12-381 G1 point from x and sign bit.
    /// Inputs: `x` is Fp-sized x (big-endian); `sign` selects the y root.
    /// Output: affine x||y encoding.
    fn bls12381_decompress(x: [u8; BLS12381_G1_COMPRESSED_SIZE], sign: u32) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];

    /// Point doubling on BLS12-381 G1.
    /// Input: affine x||y; Output: affine x||y.
    fn bls12381_double(p: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE];

    /// Add two BN254 G1 points (affine x||y, 64 bytes total).
    /// Returns the affine sum encoded as x||y.
    fn bn254_add(p: [u8; BN254_G1_RAW_AFFINE_SIZE], q: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE];

    /// Point doubling on BN254 G1.
    /// Input: affine x||y; Output: affine x||y.
    fn bn254_double(p: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE];

    /// Compute (x * y) mod m for 256-bit integers.
    /// Inputs: `x`, `y`, `m` are 32-byte little-endian;
    /// Output: 32-byte little-endian result in [0, m).
    fn uint256_mul_mod(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32];

    /// Multiply a 256-bit integer by a 2048-bit integer.
    ///
    /// Inputs: `a` is 32-byte little-endian; `b` is 256-byte little-endian.
    /// Output: (lo, hi) where `lo` is the least-significant 2048-bit limb (256 bytes),
    /// and `hi` is the top 256-bit carry (32 bytes), both little-endian.
    fn uint256_x2048_mul(a: &[u8; 32], b: &[u8; 256]) -> ([u8; 256], [u8; 32]);
}

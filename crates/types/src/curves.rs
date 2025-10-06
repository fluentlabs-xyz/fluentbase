/// ED25519
pub const ED25519_POINT_COMPRESSED_SIZE: usize = 32;
pub const ED25519_POINT_DECOMPRESSED_SIZE: usize = 64;

/// SECP256K1
pub const SECP256K1_FP_SIZE: usize = 32;
pub const SECP256K1_G1_RAW_AFFINE_SIZE: usize = 64;
pub const SECP256K1_G1_COMPRESSED_SIZE: usize = 32;

/// SECP256R1
pub const SECP256R1_FP_SIZE: usize = 32;
pub const SECP256R1_G1_RAW_AFFINE_SIZE: usize = 64;
pub const SECP256R1_G1_COMPRESSED_SIZE: usize = 33;

/// BN254
pub const BN254_FP_SIZE: usize = 32;
pub const BN254_FP2_SIZE: usize = 64;
pub const BN254_G1_RAW_AFFINE_SIZE: usize = 64;
pub const BN254_G1_COMPRESSED_SIZE: usize = 32;
pub const BN254_G2_RAW_AFFINE_SIZE: usize = 128;
pub const BN254_G2_COMPRESSED_SIZE: usize = 64;

/// BLS12381
pub const BLS12381_FP_SIZE: usize = 48;
pub const BLS12381_FP2_SIZE: usize = 96;
pub const BLS12381_G1_RAW_AFFINE_SIZE: usize = 96;
pub const BLS12381_G1_COMPRESSED_SIZE: usize = 48;
pub const BLS12381_G2_RAW_AFFINE_SIZE: usize = 192;
pub const BLS12381_G2_COMPRESSED_SIZE: usize = 96;

pub const SCALAR_SIZE: usize = 32;
/// FQ_SIZE specifies the number of bytes needed to represent an
/// Fq element. This is an element in the base field of BN254.
///
/// Note: The base field is used to define G1 and G2 elements.
pub const FQ_SIZE: usize = 32;

/// FQ2_SIZE specifies the number of bytes needed to represent a Fq2 element.
/// This is an element in the quadratic extension field of BN254.
///
/// Note: A Fq2 element contains 2 Fq elements.
pub const FQ2_SIZE: usize = 2 * FQ_SIZE;

/// BN254 Specific Constants
pub const BN254_G1_POINT_COMPRESSED_SIZE: usize = 32;
pub const BN254_G1_POINT_DECOMPRESSED_SIZE: usize = 64;
pub const BN254_G2_POINT_COMPRESSED_SIZE: usize = 64;
pub const BN254_G2_POINT_DECOMPRESSED_SIZE: usize = 128;
pub const BN254_MUL_INPUT_SIZE: usize = BN254_G1_POINT_DECOMPRESSED_SIZE + SCALAR_SIZE;
pub const BN254_ADD_INPUT_SIZE: usize = 2 * BN254_G1_POINT_DECOMPRESSED_SIZE;
pub const BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN: usize =
    BN254_G1_POINT_DECOMPRESSED_SIZE + BN254_G2_POINT_DECOMPRESSED_SIZE;

pub const CURVE256R1_POINT_COMPRESSED_SIZE: usize = 32;
pub const CURVE256R1_POINT_DECOMPRESSED_SIZE: usize = 64;

/// BLS12-381 Specific Constants
pub const FP_PAD_BY: usize = 16;
pub const FP_SIZE: usize = 48;
pub const PADDED_FP_SIZE: usize = 64;
pub const FP2_SIZE: usize = 2 * FP_SIZE;
pub const PADDED_FP2_SIZE: usize = 2 * PADDED_FP_SIZE;
pub const PADDED_G1_SIZE: usize = 2 * PADDED_FP_SIZE;
pub const PADDED_G2_SIZE: usize = 2 * PADDED_FP2_SIZE;

pub const G1_UNCOMPRESSED_SIZE: usize = 96;
pub const G1_COMPRESSED_SIZE: usize = 48;
pub const G2_UNCOMPRESSED_SIZE: usize = 192;
pub const G2_COMPRESSED_SIZE: usize = 96;
pub const GT_COMPRESSED_SIZE: usize = 288;

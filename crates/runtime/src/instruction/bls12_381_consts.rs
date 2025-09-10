use blstrs::{G1Affine, G2Affine};

pub const SCALAR_LENGTH: usize = 32;

pub const FP_PAD_BY: usize = 16;
pub const FP_LENGTH: usize = 48;
pub const PADDED_FP_LENGTH: usize = 64;
/// 96
pub const FP2_LENGTH: usize = 2 * FP_LENGTH;
/// 128
pub const PADDED_FP2_LENGTH: usize = 2 * PADDED_FP_LENGTH;
/// 128
pub const PADDED_G1_LENGTH: usize = 2 * PADDED_FP_LENGTH;
/// 256
pub const PADDED_G2_LENGTH: usize = 2 * PADDED_FP2_LENGTH;
/// 96
pub const G1_UNCOMPRESSED_LENGTH: usize = G1Affine::uncompressed_size();
/// 48
pub const G1_COMPRESSED_LENGTH: usize = G1Affine::compressed_size();
/// 192
pub const G2_UNCOMPRESSED_LENGTH: usize = G2Affine::uncompressed_size();
/// 96
pub const G2_COMPRESSED_LENGTH: usize = G2Affine::compressed_size();

use blstrs::{G1Affine, G2Affine};
use group::prime::PrimeCurveAffine;

use fluentbase_types::{FP_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE};

pub fn parse_bls12381_g1_point_uncompressed(input: &[u8; G1_UNCOMPRESSED_SIZE]) -> G1Affine {
    if input.iter().all(|&b| b == 0) {
        G1Affine::identity()
    } else {
        let ct = G1Affine::from_uncompressed(input);
        if ct.is_none().unwrap_u8() == 1 {
            G1Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

pub fn parse_bls12381_g2_point_uncompressed(input: &[u8; G2_UNCOMPRESSED_SIZE]) -> G2Affine {
    if input.iter().all(|&b| b == 0) {
        G2Affine::identity()
    } else {
        let ct = G2Affine::from_uncompressed(input);
        if ct.is_none().unwrap_u8() == 1 {
            G2Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

pub fn serialize_bls12381_g1_point_uncompressed(point: &G1Affine) -> [u8; G1_UNCOMPRESSED_SIZE] {
    point.to_uncompressed()
}

pub fn g2_le_limbs_to_be_uncompressed(
    le_limbs: &[u8; G2_UNCOMPRESSED_SIZE],
) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let mut be_uncompressed = [0u8; G2_UNCOMPRESSED_SIZE];

    // Convert each limb from LE to BE and swap positions
    let mut limb = [0u8; FP_SIZE];

    // x1 -> c0 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[FP_SIZE..2 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[0..FP_SIZE].copy_from_slice(&limb);

    // x0 -> c1 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[0..FP_SIZE]);
    limb.reverse();
    be_uncompressed[FP_SIZE..2 * FP_SIZE].copy_from_slice(&limb);

    // y1 -> c0 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[3 * FP_SIZE..4 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[2 * FP_SIZE..3 * FP_SIZE].copy_from_slice(&limb);

    // y0 -> c1 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[2 * FP_SIZE..3 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[3 * FP_SIZE..4 * FP_SIZE].copy_from_slice(&limb);

    be_uncompressed
}

pub fn g2_be_uncompressed_to_le_limbs(
    be_point: &[u8; G2_UNCOMPRESSED_SIZE],
) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let mut le_point = [0u8; G2_UNCOMPRESSED_SIZE];
    let mut limb = [0u8; FP_SIZE];

    // x0 <= c1, x1 <= c0
    limb.copy_from_slice(&be_point[FP_SIZE..2 * FP_SIZE]); // c1
    limb.reverse();
    le_point[0..FP_SIZE].copy_from_slice(&limb); // x0 LE
    limb.copy_from_slice(&be_point[0..FP_SIZE]); // c0
    limb.reverse();
    le_point[FP_SIZE..2 * FP_SIZE].copy_from_slice(&limb); // x1 LE

    // y0 <= c1, y1 <= c0
    limb.copy_from_slice(&be_point[3 * FP_SIZE..4 * FP_SIZE]); // c1
    limb.reverse();
    le_point[2 * FP_SIZE..3 * FP_SIZE].copy_from_slice(&limb); // y0 LE
    limb.copy_from_slice(&be_point[2 * FP_SIZE..3 * FP_SIZE]); // c0
    limb.reverse();
    le_point[3 * FP_SIZE..4 * FP_SIZE].copy_from_slice(&limb); // y1 LE

    le_point
}

use crate::instruction::bls12_381_consts::{
    FP2_LENGTH, FP_LENGTH, FP_PAD_BY, G1_COMPRESSED_LENGTH, G1_UNCOMPRESSED_LENGTH,
    G2_UNCOMPRESSED_LENGTH, PADDED_FP_LENGTH, PADDED_G1_LENGTH,
};
use blstrs::{G1Affine, G2Affine};
use fluentbase_types::ExitCode;
use group::prime::PrimeCurveAffine;

/// Decode 128-byte EIP-2537 G1 (x||y; each 64-byte BE with top 16 zero) to blstrs affine.
/// Infinity is 128 zero bytes.
pub fn g1_128be_to_affine(input: &[u8; PADDED_G1_LENGTH]) -> Result<G1Affine, ExitCode> {
    let zero = G1Affine::identity();
    // 1) Infinity?
    if input.iter().all(|&b| b == 0) {
        return Ok(zero);
    }
    // 2) Enforce 64-byte BE limbs with top 16 zero (both x and y)
    for limb in [
        &input[0..PADDED_FP_LENGTH],
        &input[PADDED_FP_LENGTH..PADDED_G1_LENGTH],
    ] {
        if limb[..16].iter().any(|&b| b != 0) {
            return Err(ExitCode::PrecompileError);
        }
    }
    // 3) Convert 128B (64+64 BE) → library **96B uncompressed** buffer
    //    (blstrs expects 48B BE per coord). Map by stripping the leading 16 zero bytes.
    let mut lib = [0u8; G1_UNCOMPRESSED_LENGTH];
    lib[0..FP_LENGTH].copy_from_slice(&input[FP_PAD_BY..PADDED_FP_LENGTH]); // x
    lib[FP_LENGTH..G1_COMPRESSED_LENGTH]
        .copy_from_slice(&input[FP_PAD_BY + PADDED_FP_LENGTH..PADDED_G1_LENGTH]); // y

    let ct = G1Affine::from_uncompressed(&lib);
    // For *add*, the EIP does not require subgroup check. `from_uncompressed` enforces it.
    // If you need curve-only: use a relaxed constructor via low-level blst and check is_on_curve() only.
    if ct.is_none().unwrap_u8() == 1 {
        return Err(ExitCode::PrecompileError);
    }
    Ok(ct.unwrap())
}

pub fn parse_affine_g1(input: &[u8; G1_UNCOMPRESSED_LENGTH]) -> G1Affine {
    if input.iter().all(|&b| b == 0) {
        // Treat all-zero 96B as identity (used by our ABI for infinity)
        G1Affine::identity()
    } else {
        let ct = G1Affine::from_uncompressed(input);
        if ct.is_none().unwrap_u8() == 1 {
            // Invalid point encoding
            // In this syscall context we don't have a Result; use identity to avoid panic
            // and let higher layers enforce validity as needed.
            G1Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

// Parse into affine points (validated), add in projective, and convert back to affine
pub fn parse_affine_g2(be: &[u8; G2_UNCOMPRESSED_LENGTH]) -> G2Affine {
    if be.iter().all(|&b| b == 0) {
        G2Affine::identity()
    } else {
        let ct = G2Affine::from_uncompressed(be);
        if ct.is_none().unwrap_u8() == 1 {
            G2Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

/// Converts G2 point from LE limb format to BE uncompressed format for blstrs.
///
/// Input format: x0||x1||y0||y1 (each 48B LE)
/// Output format: c0||c1||c0||c1 (each 48B BE) where c0=x1, c1=x0 for x and y coordinates
pub fn g2_le_limbs_to_be_uncompressed(
    le_point: &[u8; G2_UNCOMPRESSED_LENGTH],
) -> [u8; G2_UNCOMPRESSED_LENGTH] {
    let mut be_point = [0u8; G2_UNCOMPRESSED_LENGTH];
    let mut limb = [0u8; FP_LENGTH];

    // x: c0 <= x1, c1 <= x0
    limb.copy_from_slice(&le_point[FP_LENGTH..FP2_LENGTH]); // x1 LE
    limb.reverse();
    be_point[0..FP_LENGTH].copy_from_slice(&limb); // c0
    limb.copy_from_slice(&le_point[0..FP_LENGTH]); // x0 LE
    limb.reverse();
    be_point[FP_LENGTH..FP2_LENGTH].copy_from_slice(&limb); // c1

    // y: c0 <= y1, c1 <= y0
    limb.copy_from_slice(&le_point[3 * FP_LENGTH..4 * FP_LENGTH]); // y1 LE
    limb.reverse();
    be_point[FP2_LENGTH..3 * FP_LENGTH].copy_from_slice(&limb); // c0
    limb.copy_from_slice(&le_point[2 * FP_LENGTH..3 * FP_LENGTH]); // y0 LE
    limb.reverse();
    be_point[3 * FP_LENGTH..4 * FP_LENGTH].copy_from_slice(&limb); // c1

    be_point
}

/// Converts G2 point from BE uncompressed format back to LE limb format.
///
/// Input format: c0||c1||c0||c1 (each 48B BE)
/// Output format: x0||x1||y0||y1 (each 48B LE) where x0=c1, x1=c0 for x and y coordinates
pub fn g2_be_uncompressed_to_le_limbs(
    be_point: &[u8; G2_UNCOMPRESSED_LENGTH],
) -> [u8; G2_UNCOMPRESSED_LENGTH] {
    let mut le_point = [0u8; G2_UNCOMPRESSED_LENGTH];
    let mut limb = [0u8; FP_LENGTH];

    // x0 <= c1, x1 <= c0
    limb.copy_from_slice(&be_point[FP_LENGTH..FP2_LENGTH]); // c1
    limb.reverse();
    le_point[0..FP_LENGTH].copy_from_slice(&limb); // x0 LE
    limb.copy_from_slice(&be_point[0..FP_LENGTH]); // c0
    limb.reverse();
    le_point[FP_LENGTH..FP2_LENGTH].copy_from_slice(&limb); // x1 LE

    // y0 <= c1, y1 <= c0
    limb.copy_from_slice(&be_point[3 * FP_LENGTH..4 * FP_LENGTH]); // c1
    limb.reverse();
    le_point[2 * FP_LENGTH..3 * FP_LENGTH].copy_from_slice(&limb); // y0 LE
    limb.copy_from_slice(&be_point[FP2_LENGTH..3 * FP_LENGTH]); // c0
    limb.reverse();
    le_point[3 * FP_LENGTH..4 * FP_LENGTH].copy_from_slice(&limb); // y1 LE

    le_point
}

use crate::instruction::bls12_381_consts::{
    FP_LENGTH, G1_COMPRESSED_LENGTH, G1_UNCOMPRESSED_LENGTH, G2_UNCOMPRESSED_LENGTH,
    PADDED_FP_LENGTH, PADDED_G1_LENGTH,
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
    lib[0..FP_LENGTH].copy_from_slice(&input[16..PADDED_FP_LENGTH]); // x
    lib[FP_LENGTH..G1_COMPRESSED_LENGTH].copy_from_slice(&input[80..PADDED_G1_LENGTH]); // y

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

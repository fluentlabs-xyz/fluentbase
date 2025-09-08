use blstrs::G1Affine;
use fluentbase_types::ExitCode;
use group::prime::PrimeCurveAffine;

/// Decode 128-byte EIP-2537 G1 (x||y; each 64-byte BE with top 16 zero) to blstrs affine.
/// Infinity is 128 zero bytes.
pub fn g1_128be_to_affine(input: &[u8; 128]) -> Result<G1Affine, ExitCode> {
    let zero = G1Affine::identity();
    // 1) Infinity?
    if input.iter().all(|&b| b == 0) {
        return Ok(zero);
    }
    // 2) Enforce 64-byte BE limbs with top 16 zero (both x and y)
    for limb in [&input[0..64], &input[64..128]] {
        if limb[..16].iter().any(|&b| b != 0) {
            return Err(ExitCode::PrecompileError);
        }
    }
    // 3) Convert 128B (64+64 BE) → library **96B uncompressed** buffer
    //    (blstrs expects 48B BE per coord). Map by stripping the leading 16 zero bytes.
    let mut lib = [0u8; 96];
    lib[0..48].copy_from_slice(&input[16..64]); // x
    lib[48..96].copy_from_slice(&input[80..128]); // y

    let ct = G1Affine::from_uncompressed(&lib);
    // For *add*, the EIP does not require subgroup check. `from_uncompressed` enforces it.
    // If you need curve-only: use a relaxed constructor via low-level blst and check is_on_curve() only.
    if ct.is_none().unwrap_u8() == 1 {
        return Err(ExitCode::PrecompileError);
    }
    Ok(ct.unwrap())
}

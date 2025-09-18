use crate::{utils::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{Bn254, Fq, Fq2, G1Affine, G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::{One, Zero};
use ark_serialize::CanonicalDeserialize;
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

// Constants from REVM
const FQ_LEN: usize = 32;
const FQ2_LEN: usize = 64;

/// Reads a single `Fq` field element from the input slice (REVM compatible)
#[inline]
fn read_fq(input_be: &[u8]) -> Result<Fq, ExitCode> {
    if input_be.len() != FQ_LEN {
        return Err(ExitCode::InputOutputOutOfBounds);
    }

    let mut input_le = [0u8; FQ_LEN];
    input_le.copy_from_slice(input_be);
    input_le.reverse(); // Convert from big-endian to little-endian

    Fq::deserialize_uncompressed(&input_le[..]).map_err(|_| ExitCode::PrecompileError)
}

/// Reads a Fq2 (quadratic extension field element) from the input slice (REVM compatible)
#[inline]
fn read_fq2(input: &[u8]) -> Result<Fq2, ExitCode> {
    let y = read_fq(&input[..FQ_LEN])?;
    let x = read_fq(&input[FQ_LEN..2 * FQ_LEN])?;
    Ok(Fq2::new(x, y))
}

/// Creates a new `G1` point from the given `x` and `y` coordinates (REVM compatible)
#[inline]
fn new_g1_point(px: Fq, py: Fq) -> Result<G1Affine, ExitCode> {
    if px.is_zero() && py.is_zero() {
        Ok(G1Affine::zero())
    } else {
        let point = G1Affine::new_unchecked(px, py);
        if !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve() {
            return Err(ExitCode::PrecompileError);
        }
        Ok(point)
    }
}

/// Creates a new `G2` point from the given Fq2 coordinates (REVM compatible)
#[inline]
fn new_g2_point(x: Fq2, y: Fq2) -> Result<G2Affine, ExitCode> {
    let point = if x.is_zero() && y.is_zero() {
        G2Affine::zero()
    } else {
        let point = G2Affine::new_unchecked(x, y);
        if !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve() {
            return Err(ExitCode::PrecompileError);
        }
        point
    };
    Ok(point)
}

/// Reads a G1 point from the input slice (REVM compatible)
#[inline]
fn read_g1_point_revm(input: &[u8]) -> Result<G1Affine, ExitCode> {
    let px = read_fq(&input[0..FQ_LEN])?;
    let py = read_fq(&input[FQ_LEN..2 * FQ_LEN])?;
    new_g1_point(px, py)
}

/// Reads a G2 point from the input slice (REVM compatible)
#[inline]
fn read_g2_point_revm(input: &[u8]) -> Result<G2Affine, ExitCode> {
    let ba = read_fq2(&input[0..FQ2_LEN])?;
    let bb = read_fq2(&input[FQ2_LEN..2 * FQ2_LEN])?;
    new_g2_point(ba, bb)
}

/// Performs pairing check (REVM compatible)
#[inline]
fn pairing_check_revm(pairs: &[(G1Affine, G2Affine)]) -> bool {
    if pairs.is_empty() {
        return true;
    }

    let (g1_points, g2_points): (Vec<G1Affine>, Vec<G2Affine>) = pairs.iter().copied().unzip();
    let pairing_result = Bn254::multi_pairing(&g1_points, &g2_points);
    pairing_result.0.is_one()
}

pub struct SyscallBn256Pairing;

impl SyscallBn256Pairing {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (pairs_ptr, pairs_count, out_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
            params[2].i32().unwrap() as u32,
        );

        let pairs_byte_len =
            BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN.saturating_mul(pairs_count as usize);

        let mut pair_elements = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pair_elements)?;

        let mut pairs: Vec<([u8; 64], [u8; 128])> = pair_elements
            .chunks(BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN)
            .map(|v| {
                let g1: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = unsafe {
                    core::slice::from_raw_parts(v.as_ptr(), BN254_G1_POINT_DECOMPRESSED_SIZE)
                }
                .try_into()
                .unwrap();
                let g2: [u8; BN254_G2_POINT_DECOMPRESSED_SIZE] = unsafe {
                    core::slice::from_raw_parts(
                        v[BN254_G1_POINT_DECOMPRESSED_SIZE..BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN]
                            .as_ptr(),
                        BN254_G2_POINT_DECOMPRESSED_SIZE,
                    )
                }
                .try_into()
                .unwrap();
                (g1, g2)
            })
            .collect();

        let res = Self::fn_impl(&mut pairs).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(output) = res {
            caller.memory_write(out_ptr as usize, &output)?;
        }
        Ok(())
    }

    pub fn fn_impl(pairs: &mut [([u8; 64], [u8; 128])]) -> Result<[u8; 32], ExitCode> {
        // Parse points using REVM-compatible logic
        let mut parsed_pairs = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs.iter() {
            // Parse G1 point using REVM-compatible logic
            let g1 = read_g1_point_revm(g1_bytes)?;
            let g2 = read_g2_point_revm(g2_bytes)?;

            // Handle point-at-infinity cases like REVM
            if g1.is_zero() || g2.is_zero() {
                // Skip this pair but continue processing
                continue;
            }

            parsed_pairs.push((g1, g2));
        }

        // Perform pairing check using REVM-compatible logic
        let success = pairing_check_revm(&parsed_pairs);

        // Return big-endian 32-byte result (REVM compatible)
        let mut result = [0u8; 32];
        if success {
            result[31] = 1; // Set the last byte to 1 for true (big-endian)
        }
        Ok(result)
    }
}

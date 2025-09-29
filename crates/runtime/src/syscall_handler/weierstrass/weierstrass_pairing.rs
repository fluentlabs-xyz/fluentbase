//! BLST-based operations for BLS12-381 and other supported curves
//!
//! This module provides BLST library bindings for high-performance elliptic curve operations
//! with comprehensive error handling and edge case management.

use crate::{
    syscall_handler::{
        syscall_process_exit_code,
        weierstrass::{
            bn256_helpers::{read_g1_point, read_g2_point},
            helpers_bls::{
                parse_bls12381_g1_point_uncompressed, parse_bls12381_g2_point_uncompressed,
            },
        },
    },
    RuntimeContext,
};
use group::Group;
use rwasm::{Store, TrapCode, TypedCaller, Value};

// BN254 imports
use ark_bn254::{Bn254, G1Affine as Bn254G1Affine, G2Affine as Bn254G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::One;
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN, FP_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE,
    GT_COMPRESSED_SIZE, SCALAR_SIZE,
};

use sp1_curves::{CurveType, EllipticCurve};

/// Generic pairing handler that dispatches based on curve type
pub struct SyscallWeierstrassPairingAssign<E: EllipticCurve> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassPairingAssign<E> {
    /// Unified handler that works for all supported curve types
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (pairs_ptr, pairs_count, out_ptr) = (
            params[0].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[1].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[2].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
        );

        let pairs_byte_len =
            BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN.saturating_mul(pairs_count as usize);

        // Read pair elements from memory
        let mut pair_elements = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pair_elements)?;

        let pairs = pair_elements
            .chunks(BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN)
            .filter_map(|v| {
                if v.len() < BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN {
                    return None;
                }

                let g1_bytes = &v[0..BN254_G1_POINT_DECOMPRESSED_SIZE];
                let g2_bytes =
                    &v[BN254_G1_POINT_DECOMPRESSED_SIZE..BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN];

                let g1: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = g1_bytes.try_into().ok()?;
                let g2: [u8; BN254_G2_POINT_DECOMPRESSED_SIZE] = g2_bytes.try_into().ok()?;

                Some((g1, g2))
            })
            .collect::<Vec<(
                [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
                [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
            )>>();

        // Use the multi-pairing implementation with proper error handling
        let output =
            Self::fn_impl_multi_pairing(&pairs).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(result_data) = output {
            caller.memory_write(out_ptr as usize, &result_data)?;
        }

        Ok(())
    }

    /// Generic implementation that takes x and y byte arrays and returns pairing result
    pub fn fn_impl(x: &[u8], y: &[u8]) -> Vec<u8> {
        let mut result = vec![0u8; SCALAR_SIZE];

        match E::CURVE_TYPE {
            CurveType::Bls12381 => {
                // Parse G1 and G2 points from x and y byte arrays
                if x.len() >= G1_UNCOMPRESSED_SIZE && y.len() >= G2_UNCOMPRESSED_SIZE {
                    let g1_bytes: [u8; G1_UNCOMPRESSED_SIZE] = x[..G1_UNCOMPRESSED_SIZE]
                        .try_into()
                        .unwrap_or([0u8; G1_UNCOMPRESSED_SIZE]);
                    let g2_bytes: [u8; G2_UNCOMPRESSED_SIZE] = y[..G2_UNCOMPRESSED_SIZE]
                        .try_into()
                        .unwrap_or([0u8; G2_UNCOMPRESSED_SIZE]);

                    let g1_aff = parse_bls12381_g1_point_uncompressed(&g1_bytes);
                    let g2_aff = parse_bls12381_g2_point_uncompressed(&g2_bytes);

                    // Perform BLS12-381 pairing check
                    let pairing_check = blstrs::pairing(&g1_aff, &g2_aff);
                    let is_identity: bool = pairing_check.is_identity().into();
                    if !is_identity {
                        result[SCALAR_SIZE - 1] = 1; // Set the last byte to 1 for true (big-endian)
                    }
                }
            }
            CurveType::Bn254 => {
                // Parse G1 and G2 points from x and y byte arrays for BN254
                if x.len() >= BN254_G1_POINT_DECOMPRESSED_SIZE
                    && y.len() >= BN254_G2_POINT_DECOMPRESSED_SIZE
                {
                    let g1_bytes: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = x
                        [..BN254_G1_POINT_DECOMPRESSED_SIZE]
                        .try_into()
                        .unwrap_or([0u8; BN254_G1_POINT_DECOMPRESSED_SIZE]);
                    let g2_bytes: [u8; BN254_G2_POINT_DECOMPRESSED_SIZE] = y
                        [..BN254_G2_POINT_DECOMPRESSED_SIZE]
                        .try_into()
                        .unwrap_or([0u8; BN254_G2_POINT_DECOMPRESSED_SIZE]);

                    // Parse points using the same logic as the original bn256_pairing.rs
                    let mut parsed_pairs = Vec::new();

                    match (read_g1_point(&g1_bytes), read_g2_point(&g2_bytes)) {
                        (Ok(g1_aff), Ok(g2_aff)) => {
                            // Skip zero points (same logic as original implementation)
                            if !g1_aff.is_zero() && !g2_aff.is_zero() {
                                parsed_pairs.push((g1_aff, g2_aff));
                            }
                        }
                        _ => {
                            // Points failed to parse - leave result as all zeros
                        }
                    }

                    // If no valid pairs after filtering, return true (empty pairing is valid)
                    let success = if parsed_pairs.is_empty() {
                        true
                    } else {
                        bn254_pairing_check(&parsed_pairs)
                    };

                    if success {
                        result[SCALAR_SIZE - 1] = 1; // Set the last byte to 1 for true (big-endian)
                    }
                }
            }
            _ => {
                // Unsupported curve type - result remains all zeros
            }
        }

        result
    }

    /// Multi-pairing implementation that takes pairs of G1 and G2 points
    pub fn fn_impl_multi_pairing(
        pairs: &[(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )],
    ) -> Result<Vec<u8>, ExitCode> {
        let mut result = vec![0u8; SCALAR_SIZE];

        match E::CURVE_TYPE {
            CurveType::Bn254 => {
                // Parse points using the same logic as the original bn256_pairing.rs
                let mut parsed_pairs = Vec::new();

                for (g1_bytes, g2_bytes) in pairs.iter() {
                    let g1 =
                        read_g1_point(g1_bytes).map_err(|_| ExitCode::MalformedBuiltinParams)?;
                    let g2 =
                        read_g2_point(g2_bytes).map_err(|_| ExitCode::MalformedBuiltinParams)?;

                    // Skip zero points (same logic as original implementation)
                    if !g1.is_zero() && !g2.is_zero() {
                        parsed_pairs.push((g1, g2));
                    }
                }

                // If no valid pairs after filtering, return true (empty pairing is valid)
                let success = if parsed_pairs.is_empty() {
                    true
                } else {
                    bn254_pairing_check(&parsed_pairs)
                };

                if success {
                    result[SCALAR_SIZE - 1] = 1; // Set the last byte to 1 for true (big-endian)
                }
                Ok(result)
            }
            _ => {
                // Unsupported curve type - result remains all zeros
                Err(ExitCode::MalformedBuiltinParams)
            }
        }
    }

    /// Generic implementation for compressed format (used by context_wrapper)
    pub fn fn_impl_compressed(
        pairs: &[([u8; FP_SIZE], [u8; G1_UNCOMPRESSED_SIZE])], // Compressed sizes for BLS12-381
        out: &mut [u8; GT_COMPRESSED_SIZE],                    // GT compressed size
    ) -> Result<(), ExitCode> {
        match E::CURVE_TYPE {
            CurveType::Bls12381 => {
                // For BLS12-381, we need to decompress the points first
                let mut bls_pairs = Vec::with_capacity(pairs.len());
                for (g1_compressed, g2_compressed) in pairs.iter() {
                    // Decompress G1 point (48 bytes -> 96 bytes)
                    let mut g1_uncompressed = [0u8; G1_UNCOMPRESSED_SIZE];
                    // TODO: Implement decompression logic here
                    // For now, we'll assume the input is already in the right format
                    g1_uncompressed[..FP_SIZE].copy_from_slice(g1_compressed);

                    // Decompress G2 point (96 bytes -> 192 bytes)
                    let mut g2_uncompressed = [0u8; G2_UNCOMPRESSED_SIZE];
                    g2_uncompressed[..G1_UNCOMPRESSED_SIZE].copy_from_slice(g2_compressed);

                    let g1_aff = parse_bls12381_g1_point_uncompressed(&g1_uncompressed);
                    let g2_aff = parse_bls12381_g2_point_uncompressed(&g2_uncompressed);
                    bls_pairs.push((g1_aff, g2_aff));
                }

                // Perform BLS12-381 pairing check
                let mut pairing_result = true;
                for (g1, g2) in bls_pairs.iter() {
                    let pairing_check = blstrs::pairing(g1, g2);
                    if pairing_check.is_identity().into() {
                        pairing_result = false;
                        break;
                    }
                }

                // Set output based on result
                if pairing_result {
                    out[GT_COMPRESSED_SIZE - 1] = 1; // Set the last byte to 1 for true (big-endian)
                }
                Ok(())
            }
            _ => Err(ExitCode::UnknownError),
        }
    }
}

/// Performs pairing check on a list of G1 and G2 points for BN254.
#[inline]
fn bn254_pairing_check(pairs: &[(Bn254G1Affine, Bn254G2Affine)]) -> bool {
    if pairs.is_empty() {
        return true;
    }
    let (g1_points, g2_points): (Vec<Bn254G1Affine>, Vec<Bn254G2Affine>) =
        pairs.iter().copied().unzip();
    let pairing_result = Bn254::multi_pairing(&g1_points, &g2_points);
    pairing_result.0.is_one()
}

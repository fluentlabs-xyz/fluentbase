//! BLST-based operations for BLS12-381 and other supported curves
//!
//! This module provides BLST library bindings for high-performance elliptic curve operations
//! with comprehensive error handling and edge case management.
//!
use crate::{
    syscall_handler::{
        ecc::{
            ecc_bls12381::{
                parse_bls12381_g1_point_uncompressed, parse_bls12381_g2_point_uncompressed,
            },
            ecc_bn256::{read_g1_point, read_g2_point},
        },
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use ark_bn254::{Bn254, G1Affine as Bn254G1Affine, G2Affine as Bn254G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::One;
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN, G1_COMPRESSED_SIZE, G1_UNCOMPRESSED_SIZE,
    G2_COMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE, GT_COMPRESSED_SIZE, SCALAR_SIZE,
};
use group::Group;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{CurveType, EllipticCurve};

pub struct SyscallEccPairing<E: EllipticCurve> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EllipticCurve> SyscallEccPairing<E> {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (pairs_ptr, pairs_count, out_ptr) = (
            params[0].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[1].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[2].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
        );

        // Determine expected per-pair byte length based on curve
        let element_len = match E::CURVE_TYPE {
            CurveType::Bn254 => BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN,
            CurveType::Bls12381 => G1_UNCOMPRESSED_SIZE + G2_UNCOMPRESSED_SIZE,
            _ => BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN,
        };

        let pairs_byte_len = element_len.saturating_mul(pairs_count as usize);

        let mut pair_elements = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pair_elements)?;

        // Build pairs as (Vec<u8>, Vec<u8>)
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = pair_elements
            .chunks(element_len)
            .filter_map(|chunk| {
                if chunk.len() < element_len {
                    return None;
                }
                let (g1_len, g2_len) = match E::CURVE_TYPE {
                    CurveType::Bn254 => (
                        BN254_G1_POINT_DECOMPRESSED_SIZE,
                        BN254_G2_POINT_DECOMPRESSED_SIZE,
                    ),
                    CurveType::Bls12381 => (G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE),
                    _ => (
                        BN254_G1_POINT_DECOMPRESSED_SIZE,
                        BN254_G2_POINT_DECOMPRESSED_SIZE,
                    ),
                };
                let g1 = chunk[0..g1_len].to_vec();
                let g2 = chunk[g1_len..g1_len + g2_len].to_vec();
                Some((g1, g2))
            })
            .collect();

        let output: Result<Vec<u8>, _> =
            Self::fn_impl(&pairs).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(result_data) = output {
            caller.memory_write(out_ptr as usize, &result_data)?;
        }

        Ok(())
    }

    pub fn fn_impl(pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<u8>, ExitCode> {
        match E::CURVE_TYPE {
            CurveType::Bn254 => {
                let typed: Vec<(
                    [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
                    [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
                )> = pairs
                    .iter()
                    .filter_map(|(g1, g2)| {
                        if g1.len() < BN254_G1_POINT_DECOMPRESSED_SIZE
                            || g2.len() < BN254_G2_POINT_DECOMPRESSED_SIZE
                        {
                            return None;
                        }
                        let g1a = g1[..BN254_G1_POINT_DECOMPRESSED_SIZE].try_into().ok()?;
                        let g2a = g2[..BN254_G2_POINT_DECOMPRESSED_SIZE].try_into().ok()?;
                        Some((g1a, g2a))
                    })
                    .collect();
                Self::fn_impl_bn254(&typed)
            }
            CurveType::Bls12381 => {
                // Prefer compressed inputs; if not all compressed, try uncompressed fallback
                let all_compressed = pairs.iter().all(|(g1, g2)| {
                    g1.len() == G1_COMPRESSED_SIZE && g2.len() == G2_COMPRESSED_SIZE
                });
                if all_compressed {
                    let typed: Vec<([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])> = pairs
                        .iter()
                        .filter_map(|(g1, g2)| {
                            Some((
                                g1.as_slice().try_into().ok()?,
                                g2.as_slice().try_into().ok()?,
                            ))
                        })
                        .collect();
                    return Self::fn_impl_bls12_381(&typed);
                }

                // Uncompressed fallback
                let mut pairing_result_all_non_identity = true;
                for (g1_bytes_vec, g2_bytes_vec) in pairs.iter() {
                    if g1_bytes_vec.len() >= G1_UNCOMPRESSED_SIZE
                        && g2_bytes_vec.len() >= G2_UNCOMPRESSED_SIZE
                    {
                        let g1_bytes: [u8; G1_UNCOMPRESSED_SIZE] = g1_bytes_vec
                            [..G1_UNCOMPRESSED_SIZE]
                            .try_into()
                            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
                        let g2_bytes: [u8; G2_UNCOMPRESSED_SIZE] = g2_bytes_vec
                            [..G2_UNCOMPRESSED_SIZE]
                            .try_into()
                            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
                        let g1_aff = parse_bls12381_g1_point_uncompressed(&g1_bytes);
                        let g2_aff = parse_bls12381_g2_point_uncompressed(&g2_bytes);
                        let gt = blstrs::pairing(&g1_aff, &g2_aff);
                        let is_identity: bool = gt.is_identity().into();
                        if is_identity {
                            pairing_result_all_non_identity = false;
                            break;
                        }
                        continue;
                    }
                    pairing_result_all_non_identity = false;
                    break;
                }
                let mut out = vec![0u8; GT_COMPRESSED_SIZE];
                if pairing_result_all_non_identity {
                    out[GT_COMPRESSED_SIZE - 1] = 1;
                }
                Ok(out)
            }
            _ => Err(ExitCode::PrecompileError),
        }
    }

    pub fn fn_impl_bn254(
        pairs: &[(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )],
    ) -> Result<Vec<u8>, ExitCode> {
        let mut parsed_pairs: Vec<(Bn254G1Affine, Bn254G2Affine)> = Vec::new();
        for (g1_bytes, g2_bytes) in pairs.iter() {
            let g1 = read_g1_point(g1_bytes).map_err(|_| ExitCode::MalformedBuiltinParams)?;
            let g2 = read_g2_point(g2_bytes).map_err(|_| ExitCode::MalformedBuiltinParams)?;
            if !g1.is_zero() && !g2.is_zero() {
                parsed_pairs.push((g1, g2));
            }
        }

        let success = if parsed_pairs.is_empty() {
            true
        } else {
            bn254_pairing_check(&parsed_pairs)
        };

        let mut out = vec![0u8; SCALAR_SIZE];
        if success {
            out[SCALAR_SIZE - 1] = 1;
        }
        Ok(out)
    }

    pub fn fn_impl_bls12_381(
        pairs: &[([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])],
    ) -> Result<Vec<u8>, ExitCode> {
        let mut pairing_result_all_non_identity = true;
        for (g1c, g2c) in pairs.iter() {
            let maybe_g1 = blstrs::G1Affine::from_compressed(g1c);
            let maybe_g2 = blstrs::G2Affine::from_compressed(g2c);
            if let (Some(g1_aff), Some(g2_aff)) = (maybe_g1.into(), maybe_g2.into()) {
                let gt = blstrs::pairing(&g1_aff, &g2_aff);
                let is_identity: bool = gt.is_identity().into();
                if is_identity {
                    pairing_result_all_non_identity = false;
                    break;
                }
            } else {
                pairing_result_all_non_identity = false;
                break;
            }
        }
        let mut out = vec![0u8; GT_COMPRESSED_SIZE];
        if pairing_result_all_non_identity {
            out[GT_COMPRESSED_SIZE - 1] = 1;
        }
        Ok(out)
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

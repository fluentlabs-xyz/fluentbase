use crate::{instruction::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{Bn254, G1Affine, G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::One;
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN, SCALAR_SIZE,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

use super::bn256_helpers::{read_g1_point, read_g2_point};

pub struct SyscallBn256Pairing;

/// Performs pairing check on a list of G1 and G2 points.
#[inline]
fn pairing_check(pairs: &[(G1Affine, G2Affine)]) -> bool {
    if pairs.is_empty() {
        return true;
    }
    let (g1_points, g2_points): (Vec<G1Affine>, Vec<G2Affine>) = pairs.iter().copied().unzip();
    let pairing_result = Bn254::multi_pairing(&g1_points, &g2_points);
    pairing_result.0.is_one()
}

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

        let mut pairs: Vec<(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )> = pair_elements
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

    pub fn fn_impl(
        pairs: &mut [(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )],
    ) -> Result<[u8; SCALAR_SIZE], ExitCode> {
        // Parse points
        let mut parsed_pairs = Vec::with_capacity(pairs.len());

        for (g1_bytes, g2_bytes) in pairs.iter() {
            let g1 = read_g1_point(g1_bytes)?;
            let g2 = read_g2_point(g2_bytes)?;

            if g1.is_zero() || g2.is_zero() {
                continue;
            }

            parsed_pairs.push((g1, g2));
        }

        let success = pairing_check(&parsed_pairs);

        let mut result = [0u8; SCALAR_SIZE];
        if success {
            result[31] = 1; // Set the last byte to 1 for true (big-endian)
        }
        Ok(result)
    }
}

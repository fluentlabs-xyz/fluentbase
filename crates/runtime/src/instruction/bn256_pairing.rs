use crate::{
    instruction::weierstrass_helpers::{g1_from_decompressed_bytes, g2_from_decompressed_bytes},
    utils::syscall_process_exit_code,
    RuntimeContext,
};
use ark_bn254::{Bn254, Fq12};
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::{BigInteger, BigInteger256, One};
use fluentbase_types::{
    ExitCode, BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

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
        // Build vectors of parsed points; invalid inputs return error like revm
        let mut g1_vec = Vec::with_capacity(pairs.len());
        let mut g2_vec = Vec::with_capacity(pairs.len());
        for (g1_bytes, g2_bytes) in pairs.iter() {
            let g1 = match g1_from_decompressed_bytes(g1_bytes) {
                Ok(p) => p,
                Err(_) => return Err(ExitCode::PrecompileError),
            };
            let g2 = match g2_from_decompressed_bytes(g2_bytes) {
                Ok(p) => p,
                Err(_) => return Err(ExitCode::PrecompileError),
            };

            // Handle point-at-infinity cases like revm
            if g1.is_zero() || g2.is_zero() {
                // Skip this pair but continue processing
                continue;
            }

            g1_vec.push(g1);
            g2_vec.push(g2);
        }

        let res = Bn254::multi_pairing(g1_vec.into_iter(), g2_vec.into_iter());
        let mut out = BigInteger256::from(0u64);
        if res.0 == Fq12::one() {
            out = BigInteger256::from(1u64);
        }
        // Return little-endian 32-byte result
        let vec_bytes = out.to_bytes_le();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&vec_bytes);
        Ok(bytes)
    }
}

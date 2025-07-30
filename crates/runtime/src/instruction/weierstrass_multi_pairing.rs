use crate::RuntimeContext;
use ark_bn254::Bn254;
use ark_ff::{BigInteger, BigInteger256};
use itertools::Itertools;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use solana_bn254::{prelude::G1, target_arch::Pairing};
use sp1_curves::weierstrass::WeierstrassParameters;
use std::marker::PhantomData;

const PAIRING_ELEMENT_LEN: usize = 192;
const G1_POINT_SIZE: usize = 64;
const G2_POINT_SIZE: usize = 128;

pub struct SyscallWeierstrassMultiPairingAssign<E: WeierstrassParameters> {
    _phantom: PhantomData<E>,
}

impl<E: WeierstrassParameters> SyscallWeierstrassMultiPairingAssign<E> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

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

        let pairs_byte_len = PAIRING_ELEMENT_LEN.saturating_mul(pairs_count as usize);

        // Read p and q values from memory
        let mut pairs = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pairs)?;

        // Write the result back to memory at the p_ptr location
        let result_vec = Self::fn_impl(
            &pairs
                .chunks(PAIRING_ELEMENT_LEN)
                .map(|v| {
                    let mut g1: [u8; G1_POINT_SIZE] =
                        unsafe { core::slice::from_raw_parts(v.as_ptr(), G1_POINT_SIZE) }
                            .try_into()
                            .unwrap();
                    let g2: [u8; G2_POINT_SIZE] = unsafe {
                        core::slice::from_raw_parts(
                            v[G1_POINT_SIZE..G2_POINT_SIZE].as_ptr(),
                            G2_POINT_SIZE,
                        )
                        .try_into()
                        .unwrap()
                    };
                    (g1, g2)
                })
                .collect_vec(),
        );
        caller.memory_write(out_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(pairs: &[([u8; G1_POINT_SIZE], [u8; G2_POINT_SIZE])]) -> Vec<u8> {
        let mut vec_pairs: Vec<(ark_bn254::g1::G1Affine, ark_bn254::g2::G2Affine)> = Vec::new();
        for pair in pairs {
            vec_pairs.push((
                solana_bn254::PodG1(pair.0.clone()).try_into().unwrap(),
                solana_bn254::PodG2(pair.1.clone()).try_into().unwrap(),
            ));
        }

        let mut result = BigInteger256::from(0u64);
        let res = Bn254::multi_pairing(
            vec_pairs.iter().map(|pair| pair.0),
            vec_pairs.iter().map(|pair| pair.1),
        );

        use ark_ff::One;
        if res.0 == ark_bn254::Fq12::one() {
            result = BigInteger256::from(1u64);
        }

        result.to_bytes_le()
    }
}

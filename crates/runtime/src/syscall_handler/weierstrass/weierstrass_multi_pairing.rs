use crate::{
    syscall_handler::{g1_from_decompressed_bytes, g2_from_decompressed_bytes},
    RuntimeContext,
};
use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;
use ark_ff::{BigInteger, BigInteger256};
use fluentbase_types::{
    BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::WeierstrassParameters;
use std::marker::PhantomData;

type G1 = ark_bn254::g1::G1Affine;
type G2 = ark_bn254::g2::G2Affine;

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

        let pairs_byte_len =
            BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN.saturating_mul(pairs_count as usize);

        // Read p and q values from memory
        let mut pair_elements = vec![0u8; pairs_byte_len];
        caller.memory_read(pairs_ptr as usize, &mut pair_elements)?;

        let pairs = pair_elements
            .chunks(BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN)
            .map(|v| {
                let g1: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] = unsafe {
                    core::slice::from_raw_parts(v.as_ptr(), BN254_G1_POINT_DECOMPRESSED_SIZE)
                }
                .try_into()
                .unwrap();
                let g2: [u8; BN254_G2_POINT_DECOMPRESSED_SIZE] = unsafe {
                    core::slice::from_raw_parts(
                        v[BN254_G1_POINT_DECOMPRESSED_SIZE..BN254_G2_POINT_DECOMPRESSED_SIZE]
                            .as_ptr(),
                        BN254_G2_POINT_DECOMPRESSED_SIZE,
                    )
                    .try_into()
                    .unwrap()
                };
                (g1, g2)
            })
            .collect::<Vec<([u8; 64], [u8; 128])>>();
        let result_vec = Self::fn_impl(&pairs);
        caller.memory_write(out_ptr as usize, &result_vec)?;

        Ok(())
    }

    pub fn fn_impl(
        pairs: &[(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )],
    ) -> Vec<u8> {
        let mut vec_pairs: Vec<(G1, G2)> = Vec::new();
        for pair in pairs {
            let g1 = g1_from_decompressed_bytes(&pair.0).unwrap();
            let g2 = g2_from_decompressed_bytes(&pair.1).unwrap();
            vec_pairs.push((g1, g2));
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

use crate::RuntimeContext;
use ark_bn254::Bn254;
use ark_ec::{pairing::Pairing, AffineRepr};
use ark_ff::{BigInteger, BigInteger256};
use ark_serialize::{CanonicalDeserialize, Compress, Validate};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::WeierstrassParameters;
use std::marker::PhantomData;

const PAIRING_ELEMENT_LEN: usize = 192;
const G1_POINT_SIZE: usize = 64;
const G2_POINT_SIZE: usize = 128;

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
                .collect::<Vec<([u8; 64], [u8; 128])>>(),
        );
        caller.memory_write(out_ptr as usize, &result_vec)?;

        Ok(())
    }

    fn g1_from_bytes(bytes: [u8; G1_POINT_SIZE]) -> Result<G1, ()> {
        if bytes == [0u8; 64] {
            return Ok(G1::zero());
        }
        let g1 = G1::deserialize_with_mode(
            &*[&bytes[..], &[0u8][..]].concat(),
            Compress::No,
            Validate::Yes,
        );

        match g1 {
            Ok(g1) => {
                if !g1.is_on_curve() {
                    Err(())
                } else {
                    Ok(g1)
                }
            }
            Err(_) => Err(()),
        }
    }

    fn g2_from_bytes(bytes: [u8; G2_POINT_SIZE]) -> Result<G2, ()> {
        if bytes == [0u8; 128] {
            return Ok(G2::zero());
        }
        let g2 = G2::deserialize_with_mode(
            &*[&bytes[..], &[0u8][..]].concat(),
            Compress::No,
            Validate::Yes,
        );

        match g2 {
            Ok(g2) => {
                if !g2.is_on_curve() {
                    Err(())
                } else {
                    Ok(g2)
                }
            }
            Err(_) => Err(()),
        }
    }

    pub fn fn_impl(pairs: &[([u8; G1_POINT_SIZE], [u8; G2_POINT_SIZE])]) -> Vec<u8> {
        let mut vec_pairs: Vec<(G1, G2)> = Vec::new();
        for pair in pairs {
            // let g1_words = cast_u8_to_u32(&pair.0).unwrap();
            // let g2_words = cast_u8_to_u32(&pair.1).unwrap();
            // let g1_affine = AffinePoint::<E>::from_words_le(&g1_words);
            // let g2_affine = AffinePoint::<E>::from_words_le(&g2_words);
            let g1 = Self::g1_from_bytes(pair.0).unwrap();
            let g2 = Self::g2_from_bytes(pair.1).unwrap();
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

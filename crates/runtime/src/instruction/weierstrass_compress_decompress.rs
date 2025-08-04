use crate::{
    instruction::weierstrass_helpers::{
        g1_from_compressed_bytes,
        g1_from_decompressed_bytes,
        g2_from_compressed_bytes,
        g2_from_decompressed_bytes,
        G1_POINT_COMPRESSED_SIZE,
        G1_POINT_UNCOMPRESSED_SIZE,
        G2_POINT_COMPRESSED_SIZE,
        G2_POINT_UNCOMPRESSED_SIZE,
    },
    RuntimeContext,
};
use ark_ec::pairing::Pairing;
use ark_serialize::{CanonicalSerialize, Compress};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::EllipticCurve;
use std::marker::PhantomData;

type G1 = ark_bn254::g1::G1Affine;
type G2 = ark_bn254::g2::G2Affine;

pub struct SyscallWeierstrassCompressDecompressAssign<E: EllipticCurve> {
    _phantom: PhantomData<E>,
}

impl<E: EllipticCurve> SyscallWeierstrassCompressDecompressAssign<E> {
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
        let (pairs_ptr, out_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );

        let mut point = vec![0u8; G1_POINT_COMPRESSED_SIZE];
        caller.memory_read(pairs_ptr as usize, &mut point)?;

        // let result_vec = Self::fn_impl(&point);
        caller.memory_write(out_ptr as usize, &point)?;

        Ok(())
    }

    pub fn g1_decompress_fn_impl(
        point_compressed_bytes: &[u8; G1_POINT_COMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g1_from_compressed_bytes(point_compressed_bytes).unwrap();

        let mut point_uncompressed_bytes = [0u8; G1_POINT_UNCOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..G1_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .unwrap();
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[G1_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .unwrap();

        point_uncompressed_bytes.to_vec()
    }

    pub fn g1_compress_fn_impl(
        point_compressed_bytes: &[u8; G1_POINT_UNCOMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g1_from_decompressed_bytes(point_compressed_bytes).unwrap();

        let mut point_compressed_bytes = [0u8; G1_POINT_COMPRESSED_SIZE];

        G1::serialize_compressed(&point, point_compressed_bytes.as_mut_slice()).unwrap();

        point_compressed_bytes.to_vec()
    }

    pub fn g2_decompress_fn_impl(
        point_compressed_bytes: &[u8; G2_POINT_COMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g2_from_compressed_bytes(point_compressed_bytes).unwrap();

        let mut point_uncompressed_bytes = [0u8; G2_POINT_UNCOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..G2_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .unwrap();
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[G2_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .unwrap();

        point_uncompressed_bytes.to_vec()
    }

    pub fn g2_compress_fn_impl(
        point_compressed_bytes: &[u8; G2_POINT_UNCOMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g2_from_decompressed_bytes(point_compressed_bytes).unwrap();

        let mut point_compressed_bytes = [0u8; G2_POINT_COMPRESSED_SIZE];

        G2::serialize_compressed(&point, point_compressed_bytes.as_mut_slice()).unwrap();

        point_compressed_bytes.to_vec()
    }
}

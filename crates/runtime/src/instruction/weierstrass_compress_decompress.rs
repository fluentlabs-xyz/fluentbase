use crate::{
    instruction::weierstrass_helpers::{
        g1_from_compressed_bytes,
        g1_from_decompressed_bytes,
        g2_from_compressed_bytes,
        g2_from_decompressed_bytes,
    },
    utils::syscall_process_exit_code,
    RuntimeContext,
};
use ark_ec::pairing::Pairing;
use ark_serialize::{CanonicalSerialize, Compress};
use fluentbase_types::{
    ExitCode,
    BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};
use std::marker::PhantomData;

type G1 = ark_bn254::g1::G1Affine;

type G2 = ark_bn254::g2::G2Affine;

pub enum Curve {
    G1,
    G2,
}

pub enum Mode {
    Compress,
    Decompress,
}
pub trait Config {
    const CURVE: Curve;
    const MODE: Mode;

    fn input_point_len() -> usize {
        match Self::MODE {
            Mode::Compress => match Self::CURVE {
                Curve::G1 => BN254_G1_POINT_DECOMPRESSED_SIZE,
                Curve::G2 => BN254_G2_POINT_DECOMPRESSED_SIZE,
            },
            Mode::Decompress => match Self::CURVE {
                Curve::G1 => BN254_G1_POINT_COMPRESSED_SIZE,
                Curve::G2 => BN254_G2_POINT_COMPRESSED_SIZE,
            },
        }
    }
}

#[macro_export]
macro_rules! impl_config {
    ($curve:ty, $mode:ty) => {
        paste::paste! {
            pub struct [<Config $curve $mode >] {}
            impl Config for [<Config $curve $mode >] {
                const CURVE: Curve = Curve::$curve;
                const MODE: Mode = Mode::$mode;
            }
        }
    };
}
impl_config!(G1, Compress);
impl_config!(G2, Compress);
impl_config!(G1, Decompress);
impl_config!(G2, Decompress);

pub struct SyscallWeierstrassCompressDecompressAssign<E: Config> {
    _phantom: PhantomData<E>,
}

impl<E: Config> SyscallWeierstrassCompressDecompressAssign<E> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (point_ptr, out_ptr) = (
            params[0].i32().unwrap() as u32,
            params[1].i32().unwrap() as u32,
        );

        let point_len = E::input_point_len();
        let mut point = vec![0u8; point_len];
        caller.memory_read(point_ptr as usize, &mut point)?;

        let result_point = Self::fn_impl(&point);
        match &result_point {
            Ok(v) => {
                caller.memory_write(out_ptr as usize, v)?;
            }
            Err(error) => {
                syscall_process_exit_code(caller, *error);
            }
        }
        result[0] = Value::I32(result_point.is_err() as i32);

        Ok(())
    }

    pub fn fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let result = match E::MODE {
            Mode::Compress => match E::CURVE {
                Curve::G1 => Self::g1_compress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
                Curve::G2 => Self::g2_compress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
            },
            Mode::Decompress => match E::CURVE {
                Curve::G1 => Self::g1_decompress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
                Curve::G2 => Self::g2_decompress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
            },
        };
        Ok(result)
    }

    fn g1_decompress_fn_impl(
        point_compressed_bytes: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g1_from_compressed_bytes(point_compressed_bytes).unwrap();

        let mut point_uncompressed_bytes = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..BN254_G1_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .unwrap();
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[BN254_G1_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .unwrap();

        point_uncompressed_bytes.to_vec()
    }

    fn g1_compress_fn_impl(
        point_decompressed_bytes: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g1_from_decompressed_bytes(point_decompressed_bytes).unwrap();

        let mut point_compressed_bytes = [0u8; BN254_G1_POINT_COMPRESSED_SIZE];

        G1::serialize_compressed(&point, point_compressed_bytes.as_mut_slice()).unwrap();

        point_compressed_bytes.to_vec()
    }

    fn g2_decompress_fn_impl(
        point_compressed_bytes: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g2_from_compressed_bytes(point_compressed_bytes).unwrap();

        let mut point_uncompressed_bytes = [0u8; BN254_G2_POINT_DECOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..BN254_G2_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .unwrap();
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[BN254_G2_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .unwrap();

        point_uncompressed_bytes.to_vec()
    }

    fn g2_compress_fn_impl(
        point_decompressed_bytes: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Vec<u8> {
        let point = g2_from_decompressed_bytes(point_decompressed_bytes).unwrap();

        let mut point_compressed_bytes = [0u8; BN254_G2_POINT_COMPRESSED_SIZE];

        G2::serialize_compressed(&point, point_compressed_bytes.as_mut_slice()).unwrap();

        point_compressed_bytes.to_vec()
    }
}

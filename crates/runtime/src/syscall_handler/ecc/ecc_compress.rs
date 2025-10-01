use crate::{
    syscall_handler::{
        ecc::{
            ecc_bn256::{
                g1_from_compressed_bytes, g1_from_decompressed_bytes, g2_from_compressed_bytes,
                g2_from_decompressed_bytes,
            },
            ecc_config::{Curve, CurveConfig, Mode},
        },
        syscall_process_exit_code,
    },
    RuntimeContext,
};
use ark_serialize::{CanonicalSerialize, Compress};
use fluentbase_sdk::{
    ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    CURVE256R1_POINT_COMPRESSED_SIZE, CURVE256R1_POINT_DECOMPRESSED_SIZE, G1_COMPRESSED_SIZE,
    G1_UNCOMPRESSED_SIZE, G2_COMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE,
};
use group::prime::PrimeCurveAffine;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

// BN254 types for backward compatibility
type G1 = ark_bn254::g1::G1Affine;
type G2 = ark_bn254::g2::G2Affine;

/// Generic Weierstrass compress/decompress syscall handler
pub struct SyscallEccCompressDecompress<C: CurveConfig> {
    _phantom: PhantomData<C>,
}

impl<C: CurveConfig> SyscallEccCompressDecompress<C> {
    pub const fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let (point_ptr, out_ptr) = (
            params[0].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
            params[1].i32().ok_or(TrapCode::UnreachableCodeReached)? as u32,
        );

        let point_len = C::input_point_len();
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
        match C::CURVE_TYPE {
            CurveType::Bn254 => Self::bn254_impl(point),
            CurveType::Bls12381 => Self::bls12381_impl(point),
            CurveType::Secp256k1 => Self::secp256k1_impl(point),
            _ => Err(ExitCode::MalformedBuiltinParams),
        }
    }

    fn bn254_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        match C::MODE {
            Mode::Compress => match C::CURVE {
                Curve::G1 => Self::bn254_g1_compress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
                Curve::G2 => Self::bn254_g2_compress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
            },
            Mode::Decompress => match C::CURVE {
                Curve::G1 => Self::bn254_g1_decompress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
                Curve::G2 => Self::bn254_g2_decompress_fn_impl(
                    &point
                        .try_into()
                        .map_err(|_| ExitCode::MalformedBuiltinParams)?,
                ),
            },
        }
    }

    fn bls12381_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        match C::MODE {
            Mode::Compress => match C::CURVE {
                Curve::G1 => Self::bls12381_g1_compress_fn_impl(point),
                Curve::G2 => Self::bls12381_g2_compress_fn_impl(point),
            },
            Mode::Decompress => match C::CURVE {
                Curve::G1 => Self::bls12381_g1_decompress_fn_impl(point),
                Curve::G2 => Self::bls12381_g2_decompress_fn_impl(point),
            },
        }
    }

    fn secp256k1_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        match C::MODE {
            Mode::Compress => Self::secp256k1_compress_fn_impl(point),
            Mode::Decompress => Self::secp256k1_decompress_fn_impl(point),
        }
    }

    // BN254 implementations
    fn bn254_g1_decompress_fn_impl(
        point_compressed_bytes: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<Vec<u8>, ExitCode> {
        let point = g1_from_compressed_bytes(point_compressed_bytes)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let mut point_uncompressed_bytes = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..BN254_G1_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[BN254_G1_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        Ok(point_uncompressed_bytes.to_vec())
    }

    fn bn254_g1_compress_fn_impl(
        point_decompressed_bytes: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<Vec<u8>, ExitCode> {
        let point = g1_from_decompressed_bytes(point_decompressed_bytes)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let mut point_compressed_bytes = [0u8; BN254_G1_POINT_COMPRESSED_SIZE];

        G1::serialize_compressed(&point, point_compressed_bytes.as_mut_slice())
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        Ok(point_compressed_bytes.to_vec())
    }

    fn bn254_g2_decompress_fn_impl(
        point_compressed_bytes: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<Vec<u8>, ExitCode> {
        let point = g2_from_compressed_bytes(point_compressed_bytes)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let mut point_uncompressed_bytes = [0u8; BN254_G2_POINT_DECOMPRESSED_SIZE];

        point
            .x
            .serialize_with_mode(
                &mut point_uncompressed_bytes[..BN254_G2_POINT_COMPRESSED_SIZE],
                Compress::No,
            )
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        point
            .y
            .serialize_with_mode(
                &mut point_uncompressed_bytes[BN254_G2_POINT_COMPRESSED_SIZE..],
                Compress::No,
            )
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        Ok(point_uncompressed_bytes.to_vec())
    }

    fn bn254_g2_compress_fn_impl(
        point_decompressed_bytes: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<Vec<u8>, ExitCode> {
        let point = g2_from_decompressed_bytes(point_decompressed_bytes)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let mut point_compressed_bytes = [0u8; BN254_G2_POINT_COMPRESSED_SIZE];

        G2::serialize_compressed(&point, point_compressed_bytes.as_mut_slice())
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        Ok(point_compressed_bytes.to_vec())
    }

    // BLS12-381 implementations
    fn bls12381_g1_compress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use blstrs::G1Affine;

        if point.len() != G1_UNCOMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; G1_UNCOMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let g1_point = G1Affine::from_uncompressed(&point_array);
        if g1_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let compressed = g1_point
            .unwrap_or(blstrs::G1Affine::identity())
            .to_compressed();
        Ok(compressed.to_vec())
    }

    fn bls12381_g1_decompress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use blstrs::G1Affine;

        if point.len() != G1_COMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; G1_COMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let g1_point = G1Affine::from_compressed(&point_array);
        if g1_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let uncompressed = g1_point
            .unwrap_or(blstrs::G1Affine::identity())
            .to_uncompressed();
        Ok(uncompressed.to_vec())
    }

    fn bls12381_g2_compress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use blstrs::G2Affine;

        if point.len() != G2_UNCOMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; G2_UNCOMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let g2_point = G2Affine::from_uncompressed(&point_array);
        if g2_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let compressed = g2_point
            .unwrap_or(blstrs::G2Affine::identity())
            .to_compressed();
        Ok(compressed.to_vec())
    }

    fn bls12381_g2_decompress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use blstrs::G2Affine;

        if point.len() != G2_COMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; G2_COMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let g2_point = G2Affine::from_compressed(&point_array);
        if g2_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let uncompressed = g2_point
            .unwrap_or(blstrs::G2Affine::identity())
            .to_uncompressed();
        Ok(uncompressed.to_vec())
    }

    // Secp256k1 implementations
    fn secp256k1_compress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use k256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};

        if point.len() != CURVE256R1_POINT_DECOMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; CURVE256R1_POINT_DECOMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let encoded_point =
            k256::elliptic_curve::sec1::EncodedPoint::<k256::Secp256k1>::from_bytes(&point_array)
                .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let secp_point = k256::ProjectivePoint::from_encoded_point(&encoded_point);
        if secp_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let compressed = secp_point
            .unwrap_or(k256::ProjectivePoint::IDENTITY)
            .to_encoded_point(true);
        Ok(compressed.as_bytes().to_vec())
    }

    fn secp256k1_decompress_fn_impl(point: &[u8]) -> Result<Vec<u8>, ExitCode> {
        use k256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};

        if point.len() != CURVE256R1_POINT_COMPRESSED_SIZE {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let point_array: [u8; CURVE256R1_POINT_COMPRESSED_SIZE] = point
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let encoded_point =
            k256::elliptic_curve::sec1::EncodedPoint::<k256::Secp256k1>::from_bytes(&point_array)
                .map_err(|_| ExitCode::MalformedBuiltinParams)?;

        let secp_point = k256::ProjectivePoint::from_encoded_point(&encoded_point);
        if secp_point.is_none().unwrap_u8() == 1 {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let uncompressed = secp_point
            .unwrap_or(k256::ProjectivePoint::IDENTITY)
            .to_encoded_point(false);
        Ok(uncompressed.as_bytes().to_vec())
    }
}

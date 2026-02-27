/// Generic Weierstrass compress/decompress syscall handler
use crate::syscall_handler::syscall_process_exit_code;
use crate::RuntimeContext;
use amcl::bls381::bls381::utils::deserialize_g1;
use fluentbase_types::{
    ExitCode, BLS12381_G1_COMPRESSED_SIZE, BLS12381_G1_RAW_AFFINE_SIZE,
    SECP256K1_G1_COMPRESSED_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE,
    SECP256R1_G1_RAW_AFFINE_SIZE,
};
use k256::elliptic_curve::{point::DecompressPoint, sec1::ToEncodedPoint, subtle::Choice};
use num::{BigUint, Num};
use rwasm::{StoreTr, TrapCode, Value};
use sp1_curves::{
    weierstrass::{bls12_381::Bls12381, secp256k1::Secp256k1, secp256r1::Secp256r1},
    AffinePoint, CurveType, EllipticCurve,
};

pub fn syscall_secp256k1_decompress_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_decompress_handler::<
        Secp256k1,
        { SECP256K1_G1_COMPRESSED_SIZE },
        { SECP256K1_G1_RAW_AFFINE_SIZE },
    >(ctx, params, _result)
}
pub fn syscall_secp256r1_decompress_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_decompress_handler::<
        Secp256r1,
        { SECP256R1_G1_COMPRESSED_SIZE },
        { SECP256R1_G1_RAW_AFFINE_SIZE },
    >(ctx, params, _result)
}
pub fn syscall_bls12381_decompress_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_decompress_handler::<
        Bls12381,
        { BLS12381_G1_COMPRESSED_SIZE },
        { BLS12381_G1_RAW_AFFINE_SIZE },
    >(ctx, params, _result)
}

fn syscall_weierstrass_decompress_handler<
    E: EllipticCurve,
    const COMPRESSED_SIZE: usize,
    const DECOMPRESSED_SIZE: usize,
>(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (yx_ptr, sign_bit) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
    );

    let mut x_bytes_le = [0u8; COMPRESSED_SIZE];
    ctx.memory_read(yx_ptr + COMPRESSED_SIZE, &mut x_bytes_le)?;

    let yx_bytes_le = syscall_weierstrass_decompress_impl::<E, COMPRESSED_SIZE, DECOMPRESSED_SIZE>(
        x_bytes_le, sign_bit,
    )
    .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;

    ctx.memory_write(yx_ptr, &yx_bytes_le)?;
    Ok(())
}

/// Secp256k1 point decompression.
///
/// # Input format
/// - `x_bytes_le`: x-coordinate as 32 bytes in little-endian
/// - `sign_bit`: 0 or 1, indicates which y to recover
///
/// # Output
/// Affine point `[y || x]` in little-endian (96 bytes total, y first).
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `sign_bit > 1`
/// - x-coordinate doesn't correspond to a valid curve point
pub fn syscall_secp256k1_decompress_impl(
    x_bytes_le: [u8; SECP256K1_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; SECP256K1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Secp256k1,
        { SECP256K1_G1_COMPRESSED_SIZE },
        { SECP256K1_G1_RAW_AFFINE_SIZE },
    >(x_bytes_le, sign_bit)
}

/// Secp256r1 point decompression.
///
/// # Input format
/// - `x_bytes_le`: x-coordinate as 32 bytes in little-endian
/// - `sign_bit`: 0 or 1, indicates which y to recover
///
/// # Output
/// Affine point `[y || x]` in little-endian (64 bytes total, y first).
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `sign_bit > 1`
/// - x-coordinate doesn't correspond to a valid curve point
pub fn syscall_secp256r1_decompress_impl(
    x_bytes_le: [u8; SECP256R1_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; SECP256R1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Secp256r1,
        { SECP256R1_G1_COMPRESSED_SIZE },
        { SECP256R1_G1_RAW_AFFINE_SIZE },
    >(x_bytes_le, sign_bit)
}

/// BLS12-381 point decompression.
///
/// # Input format
/// - `x_bytes_le`: x-coordinate as 48 bytes in little-endian
/// - `sign_bit`: 0 or 1, indicates which y to recover
///
/// # Output
/// Affine point `[y || x]` in little-endian (96 bytes total, y first).
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `sign_bit > 1`
/// - x-coordinate doesn't correspond to a valid curve point
pub fn syscall_bls12381_decompress_impl(
    x_bytes_le: [u8; BLS12381_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; BLS12381_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Bls12381,
        { BLS12381_G1_COMPRESSED_SIZE },
        { BLS12381_G1_RAW_AFFINE_SIZE },
    >(x_bytes_le, sign_bit)
}

fn syscall_weierstrass_decompress_impl<
    E: EllipticCurve,
    const COMPRESSED_SIZE: usize,
    const DECOMPRESSED_SIZE: usize,
>(
    x_bytes_le: [u8; COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; DECOMPRESSED_SIZE], ExitCode> {
    if sign_bit > 1 {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    // Note: the code bellow is copied from SP1 repository as-is,
    //  where we replaced panics with error codes to avoid node crashes.
    //
    // https://github.com/succinctlabs/sp1/tree/dev/crates/curves/src/weierstrass
    let computed_point = match E::CURVE_TYPE {
        CurveType::Secp256k1 => {
            let mut x_bytes_be = x_bytes_le;
            x_bytes_be.reverse();
            let computed_point = k256::AffinePoint::decompress(
                x_bytes_be.as_slice().into(),
                Choice::from(sign_bit as u8),
            );
            if computed_point.is_none().into() {
                return Err(ExitCode::MalformedBuiltinParams);
            }
            let point = computed_point.unwrap().to_encoded_point(false);
            let x = BigUint::from_bytes_be(point.x().unwrap());
            let y = BigUint::from_bytes_be(point.y().unwrap());
            AffinePoint::<E>::new(x, y)
        }
        CurveType::Secp256r1 => {
            let mut x_bytes_be = x_bytes_le;
            x_bytes_be.reverse();
            let computed_point = p256::AffinePoint::decompress(
                x_bytes_be.as_slice().into(),
                Choice::from(sign_bit as u8),
            );
            if computed_point.is_none().into() {
                return Err(ExitCode::MalformedBuiltinParams);
            }
            let point = computed_point.unwrap().to_encoded_point(false);
            let x = BigUint::from_bytes_be(point.x().unwrap());
            let y = BigUint::from_bytes_be(point.y().unwrap());
            AffinePoint::<E>::new(x, y)
        }
        CurveType::Bls12381 => {
            let mut g1_bytes_be = x_bytes_le;
            g1_bytes_be.reverse();

            const COMPRESSION_FLAG: u8 = 0b_1000_0000;
            const Y_IS_ODD_FLAG: u8 = 0b_0010_0000;

            let mut flags = COMPRESSION_FLAG;
            if sign_bit == 1 {
                flags |= Y_IS_ODD_FLAG;
            };

            // set sign and compression flag
            g1_bytes_be[0] |= flags;
            let point =
                deserialize_g1(&g1_bytes_be).map_err(|_| ExitCode::MalformedBuiltinParams)?;

            // TODO(dmitry123): These unwraps won't fire, but it's better to operate with raw bytes,
            //  instead of hex encoding/decoding. This code is copied from SP1...
            let x_str = point.getx().to_string();
            let x = BigUint::from_str_radix(x_str.as_str(), 16).unwrap();
            let y_str = point.gety().to_string();
            let y = BigUint::from_str_radix(y_str.as_str(), 16).unwrap();

            AffinePoint::new(x, y)
        }
        _ => panic!("unsupported curve: {}", E::CURVE_TYPE),
    };

    let y_bytes_le = computed_point.y.to_bytes_le();

    let mut result_bytes_le = [0u8; DECOMPRESSED_SIZE];
    let (result_y, result_x) = result_bytes_le.split_at_mut(DECOMPRESSED_SIZE / 2);
    result_y[..y_bytes_le.len()].clone_from_slice(&y_bytes_le);
    result_x[..x_bytes_le.len()].clone_from_slice(&x_bytes_le);
    Ok(result_bytes_le)
}

#[cfg(test)]
mod tests {
    use super::*;
    use amcl::{
        bls381::bls381::{basic::key_pair_generate_g2, utils::deserialize_g1},
        rand::RAND,
    };
    use fluentbase_types::hex;
    use rand::{rng, Rng, RngCore};

    pub fn p256_decompress(compressed_key: &[u8]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        assert_eq!(compressed_key.len(), 33);
        let is_odd = match compressed_key[0] {
            2 => false,
            3 => true,
            _ => unreachable!(),
        };
        let mut compressed_key: [u8; 32] = compressed_key[1..].try_into().unwrap();
        compressed_key.reverse();
        let mut result: [u8; 64] = syscall_secp256r1_decompress_impl(compressed_key, is_odd as u32)
            .unwrap()
            .try_into()
            .unwrap();
        result.reverse();
        result
    }

    /// This test is reconstructed from SP1 sources:
    /// - sp1/crates/test-artifacts/programs/secp256r1-decompress/src/main.rs
    /// - sp1/crates/core/machine/src/syscall/precompiles/weierstrass/weierstrass_decompress.rs
    /// - sp1/crates/zkvm/entrypoint/src/syscalls/secp256r1.rs
    #[test]
    fn test_secp256r1_sp1_decompress() {
        let mut rng = rng();
        for _ in 0..100 {
            let mut random_private_key = [0u8; 32];
            rng.fill_bytes(&mut random_private_key);
            let secret_key = p256::SecretKey::from_slice(&random_private_key).unwrap();
            let public_key = secret_key.public_key();
            let decompressed = public_key.to_encoded_point(false).to_bytes();
            let compressed = public_key.to_encoded_point(true).to_bytes();
            let result = p256_decompress(&compressed);
            assert_eq!(hex::encode(result), hex::encode(&decompressed[1..]));
        }
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: MalformedBuiltinParams"
    )]
    fn test_secp256r1_bad_point() {
        let mut point: [u8; 33] = [0xff; 33];
        point[0] = 0x03;
        let _ = p256_decompress(&point);
    }

    pub fn k256_decompress(compressed_key: &[u8]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        assert_eq!(compressed_key.len(), 33);
        let is_odd = match compressed_key[0] {
            2 => false,
            3 => true,
            _ => panic!("Invalid compressed key"),
        };
        let mut compressed_key: [u8; 32] = compressed_key[1..].try_into().unwrap();
        compressed_key.reverse();
        let mut result: [u8; 64] = syscall_secp256k1_decompress_impl(compressed_key, is_odd as u32)
            .unwrap()
            .try_into()
            .unwrap();
        result.reverse();
        result
    }

    #[test]
    fn test_secp256k1_sp1_decompress() {
        let mut rng = rng();
        for _ in 0..100 {
            let mut random_private_key = [0u8; 32];
            rng.fill_bytes(&mut random_private_key);
            let secret_key = k256::SecretKey::from_slice(&random_private_key).unwrap();
            let public_key = secret_key.public_key();
            let decompressed = public_key.to_encoded_point(false).to_bytes();
            let compressed = public_key.to_encoded_point(true).to_bytes();
            let result = k256_decompress(&compressed);
            assert_eq!(hex::encode(result), hex::encode(&decompressed[1..]));
        }
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: MalformedBuiltinParams"
    )]
    fn test_secp256k1_bad_point() {
        let mut point: [u8; 33] = [0xff; 33];
        point[0] = 0x03;
        let _ = k256_decompress(&point);
    }

    pub fn bls12381_decompress(
        compressed_key: [u8; BLS12381_G1_COMPRESSED_SIZE],
    ) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        let mut compressed_key_unsigned = [0u8; BLS12381_G1_COMPRESSED_SIZE];
        compressed_key_unsigned.copy_from_slice(&compressed_key);
        let sign_bit = ((compressed_key_unsigned[0] & 0b_0010_0000) >> 5) == 1;
        compressed_key_unsigned[0] &= 0b_0001_1111;
        compressed_key_unsigned.reverse();
        let mut result: [u8; 96] =
            syscall_bls12381_decompress_impl(compressed_key_unsigned, sign_bit as u32)
                .unwrap()
                .try_into()
                .unwrap();
        result.reverse();
        result
    }

    #[test]
    fn test_bls12381_sp1_decompress() {
        let mut rng = rng();
        let mut rand = RAND::new();
        let len = 100;
        let num_tests = 10;
        let random_slice = (0..len).map(|_| rng.random::<u8>()).collect::<Vec<u8>>();
        rand.seed(len, &random_slice);
        for _ in 0..num_tests {
            let (_, compressed) = key_pair_generate_g2(&mut rand);
            let point = deserialize_g1(&compressed).unwrap();
            let x = point.getx().to_string();
            let y = point.gety().to_string();
            let result = bls12381_decompress(compressed);
            assert_eq!(
                hex::encode(result).to_lowercase(),
                format!("{x}{y}").to_lowercase()
            );
        }
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: MalformedBuiltinParams"
    )]
    fn test_bls12381_bad_point() {
        let point: [u8; 48] = [0xff; 48];
        let _ = bls12381_decompress(point);
    }
}

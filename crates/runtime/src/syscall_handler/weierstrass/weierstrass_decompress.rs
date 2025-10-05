/// Generic Weierstrass compress/decompress syscall handler
use crate::syscall_handler::syscall_process_exit_code;
use crate::RuntimeContext;
use elliptic_curve::generic_array::typenum::Unsigned;
use fluentbase_types::{
    ExitCode, BLS12381_G1_COMPRESSED_SIZE, BLS12381_G1_RAW_AFFINE_SIZE, BN254_G1_COMPRESSED_SIZE,
    BN254_G1_RAW_AFFINE_SIZE, SECP256K1_G1_COMPRESSED_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE,
    SECP256R1_G1_COMPRESSED_SIZE, SECP256R1_G1_RAW_AFFINE_SIZE,
};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{
    params::NumLimbs,
    weierstrass::{
        bls12_381::{bls12381_decompress, Bls12381},
        bn254::Bn254,
        secp256k1::{secp256k1_decompress, Secp256k1},
        secp256r1::{secp256r1_decompress, Secp256r1},
    },
    CurveType, EllipticCurve,
};

pub fn syscall_secp256k1_decompress_handler(
    ctx: &mut impl Store<RuntimeContext>,
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
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_decompress_handler::<
        Secp256r1,
        { SECP256R1_G1_COMPRESSED_SIZE },
        { SECP256R1_G1_RAW_AFFINE_SIZE },
    >(ctx, params, _result)
}
pub fn syscall_bn254_decompress_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_decompress_handler::<
        Bn254,
        { BN254_G1_COMPRESSED_SIZE },
        { BN254_G1_RAW_AFFINE_SIZE },
    >(ctx, params, _result)
}
pub fn syscall_bls12381_decompress_handler(
    ctx: &mut impl Store<RuntimeContext>,
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
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (slice_ptr, sign_bit) = (
        params[0].i32().unwrap() as usize,
        params[1].i32().unwrap() as u32,
    );

    let mut x_bytes = [0u8; COMPRESSED_SIZE];
    ctx.memory_read(slice_ptr + COMPRESSED_SIZE, &mut x_bytes)?;

    let decompressed_y_bytes =
        syscall_weierstrass_decompress_impl::<E, COMPRESSED_SIZE, DECOMPRESSED_SIZE>(
            x_bytes, sign_bit,
        )
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;

    ctx.memory_write(slice_ptr, &decompressed_y_bytes)?;
    Ok(())
}

pub fn syscall_secp256k1_decompress_impl(
    x_bytes: [u8; SECP256K1_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; SECP256K1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Secp256k1,
        { SECP256K1_G1_COMPRESSED_SIZE },
        { SECP256K1_G1_RAW_AFFINE_SIZE },
    >(x_bytes, sign_bit)
}
pub fn syscall_secp256r1_decompress_impl(
    x_bytes: [u8; SECP256R1_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; SECP256R1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Secp256r1,
        { SECP256R1_G1_COMPRESSED_SIZE },
        { SECP256R1_G1_RAW_AFFINE_SIZE },
    >(x_bytes, sign_bit)
}
pub fn syscall_bn254_decompress_impl(
    x_bytes: [u8; BN254_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; BN254_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Bn254,
        { BN254_G1_COMPRESSED_SIZE },
        { BN254_G1_RAW_AFFINE_SIZE },
    >(x_bytes, sign_bit)
}
pub fn syscall_bls12381_decompress_impl(
    x_bytes: [u8; BLS12381_G1_COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; BLS12381_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_decompress_impl::<
        Bls12381,
        { BLS12381_G1_COMPRESSED_SIZE },
        { BLS12381_G1_RAW_AFFINE_SIZE },
    >(x_bytes, sign_bit)
}

fn syscall_weierstrass_decompress_impl<
    E: EllipticCurve,
    const COMPRESSED_SIZE: usize,
    const DECOMPRESSED_SIZE: usize,
>(
    x_bytes: [u8; COMPRESSED_SIZE],
    sign_bit: u32,
) -> Result<[u8; DECOMPRESSED_SIZE], ExitCode> {
    let num_bytes = <E::BaseField as NumLimbs>::Limbs::USIZE * 4;
    if sign_bit > 1 {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let mut x_bytes = x_bytes.to_vec();
    x_bytes.reverse();

    let decompress_fn = match E::CURVE_TYPE {
        CurveType::Secp256k1 => secp256k1_decompress::<E>,
        CurveType::Secp256r1 => secp256r1_decompress::<E>,
        CurveType::Bls12381 => bls12381_decompress::<E>,
        _ => panic!("unsupported curve: {}", E::CURVE_TYPE),
    };

    let computed_point = decompress_fn(&x_bytes, sign_bit);

    let mut decompressed_y_bytes = computed_point.y.to_bytes_le();
    decompressed_y_bytes.resize(num_bytes, 0u8);
    Ok(decompressed_y_bytes.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    // use crate::syscall_handler::syscall_bls12381_decompress_impl;
    //
    // pub fn decompress_bls12381_pubkey(compressed_key: &[u8; 48]) -> [u8; 96] {
    //     let mut compressed_key_unsigned = [0u8; 96];
    //     compressed_key_unsigned[..48].copy_from_slice(compressed_key);
    //     let sign_bit = ((compressed_key_unsigned[0] & 0b_0010_0000) >> 5) == 1;
    //     compressed_key_unsigned[0] &= 0b_0001_1111;
    //     syscall_bls12381_decompress_impl(&compressed_key_unsigned, sign_bit as u32)
    //         .unwrap()
    //         .try_into()
    //         .unwrap()
    // }

    pub fn test_bls12381_decompress() {}
}

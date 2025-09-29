//! Helper functions for Weierstrass curve operations, BN254 specific
//!
//! This module provides utility functions for point parsing, validation, format conversions,
//! and other common operations across different Weierstrass curves.

use ark_ec::AffineRepr;
use ark_serialize::{CanonicalDeserialize, Compress, Validate};
use fluentbase_types::{
    BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
};

type G1 = ark_bn254::g1::G1Affine;
type G2 = ark_bn254::g2::G2Affine;

pub fn g1_from_decompressed_bytes(
    bytes: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
) -> Result<G1, ()> {
    if *bytes == [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
        return Ok(G1::zero());
    }
    let reader = &bytes[..];
    let affine = G1::deserialize_with_mode(reader, Compress::No, Validate::Yes);

    match affine {
        Ok(affine) => {
            if !affine.is_on_curve() {
                Err(())
            } else {
                Ok(affine)
            }
        }
        Err(_) => Err(()),
    }
}

pub fn g1_from_compressed_bytes(bytes: &[u8; BN254_G1_POINT_COMPRESSED_SIZE]) -> Result<G1, ()> {
    if *bytes == [0u8; BN254_G1_POINT_COMPRESSED_SIZE] {
        return Ok(G1::zero());
    }
    let reader = &bytes[..];
    let affine = G1::deserialize_with_mode(reader, Compress::Yes, Validate::Yes);

    match affine {
        Ok(affine) => {
            if !affine.is_on_curve() {
                Err(())
            } else {
                Ok(affine)
            }
        }
        Err(_) => Err(()),
    }
}

pub fn g2_from_decompressed_bytes(
    bytes: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
) -> Result<G2, ()> {
    if *bytes == [0u8; BN254_G2_POINT_DECOMPRESSED_SIZE] {
        return Ok(G2::zero());
    }
    let reader = &bytes[..];
    let affine = G2::deserialize_with_mode(reader, Compress::No, Validate::Yes);

    match affine {
        Ok(affine) => {
            if !affine.is_on_curve() {
                Err(())
            } else {
                Ok(affine)
            }
        }
        Err(_) => Err(()),
    }
}

pub fn g2_from_compressed_bytes(bytes: &[u8; BN254_G2_POINT_COMPRESSED_SIZE]) -> Result<G2, ()> {
    if *bytes == [0u8; BN254_G2_POINT_COMPRESSED_SIZE] {
        return Ok(G2::zero());
    }
    let reader = &bytes[..];
    let affine = G2::deserialize_with_mode(reader, Compress::Yes, Validate::Yes);

    match affine {
        Ok(affine) => {
            if !affine.is_on_curve() {
                Err(())
            } else {
                Ok(affine)
            }
        }
        Err(_) => Err(()),
    }
}

/// Constant-time check for zero point to prevent timing attacks
pub fn is_zero_point(data: &[u8]) -> bool {
    use elliptic_curve::subtle::ConstantTimeEq;

    // Use constant-time comparison to prevent timing attacks
    let mut result = 0u8;
    for &byte in data {
        result |= byte;
    }
    result.ct_eq(&0u8).into()
}

pub fn be_xy_to_le_words64(
    input: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
) -> [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
    const FQ_SIZE: usize = BN254_G1_POINT_DECOMPRESSED_SIZE / 2;
    let mut out = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
    for i in 0..FQ_SIZE {
        out[i] = input[FQ_SIZE - 1 - i];
    }
    for i in 0..FQ_SIZE {
        out[FQ_SIZE + i] = input[BN254_G1_POINT_DECOMPRESSED_SIZE - 1 - i];
    }
    out
}

pub fn le_words64_to_be_xy(
    input: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
) -> [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
    const FQ_SIZE: usize = BN254_G1_POINT_DECOMPRESSED_SIZE / 2;
    let mut out = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
    for i in 0..FQ_SIZE {
        out[i] = input[FQ_SIZE - 1 - i];
    }
    for i in 0..FQ_SIZE {
        out[FQ_SIZE + i] = input[BN254_G1_POINT_DECOMPRESSED_SIZE - 1 - i];
    }
    out
}

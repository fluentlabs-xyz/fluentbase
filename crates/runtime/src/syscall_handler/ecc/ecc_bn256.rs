pub use ark_bn254::{Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::AffineRepr;
use ark_ff::{PrimeField, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, Compress, Validate};
use fluentbase_sdk::{
    ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, FQ2_SIZE, FQ_SIZE,
    SCALAR_SIZE,
};

/// Reads a scalar from the input slice.
#[inline]
pub fn read_scalar(input_be: &[u8]) -> Fr {
    // Interpret as big-endian bytes and reduce modulo r
    let mut be = [0u8; SCALAR_SIZE];
    be.copy_from_slice(input_be);
    Fr::from_be_bytes_mod_order(&be)
}

/// Reads a single `Fq` field element from the input slice
#[inline]
pub fn read_fq(input_be: &[u8]) -> Result<Fq, ExitCode> {
    if input_be.len() != FQ_SIZE {
        return Err(ExitCode::InputOutputOutOfBounds);
    }

    let mut input_le = [0u8; FQ_SIZE];
    input_le.copy_from_slice(input_be);
    input_le.reverse(); // Convert from big-endian to little-endian

    Fq::deserialize_uncompressed(&input_le[..]).map_err(|_| ExitCode::PrecompileError)
}

/// Reads Fq2 (quadratic extension field element) from the input slice
#[inline]
pub fn read_fq2(input: &[u8]) -> Result<Fq2, ExitCode> {
    let y = read_fq(&input[..FQ_SIZE])?;
    let x = read_fq(&input[FQ_SIZE..2 * FQ_SIZE])?;
    Ok(Fq2::new(x, y))
}

/// Reads a G1 point from the input slice.
///
/// Parses a G1 point from a byte slice by reading two consecutive field elements
/// representing the x and y coordinates.
///
/// # Panics
///
/// Panics if the input is not at least 64 bytes long.
#[inline]
pub fn read_g1_point(input: &[u8]) -> Result<G1Affine, ExitCode> {
    let px = read_fq(&input[0..FQ_SIZE])?;
    let py = read_fq(&input[FQ_SIZE..2 * FQ_SIZE])?;
    new_g1_point(px, py)
}

/// Creates a new `G1` point from the given `x` and `y` coordinates.
///
/// Constructs a point on the G1 curve from its affine coordinates.
///
/// Note: The point at infinity which is represented as (0,0) is
/// handled specifically because `AffineG1` is not capable of
/// representing such a point.
/// In particular, when we convert from `AffineG1` to `G1`, the point
/// will be (0,0,1) instead of (0,1,0)
#[inline]
pub fn new_g1_point(px: Fq, py: Fq) -> Result<G1Affine, ExitCode> {
    if px.is_zero() && py.is_zero() {
        Ok(G1Affine::zero())
    } else {
        // We cannot use `G1Affine::new` because that triggers assert if the point is not on the curve.
        let point = G1Affine::new_unchecked(px, py);
        if !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve() {
            return Err(ExitCode::PrecompileError);
        }
        Ok(point)
    }
}

#[inline]
pub fn read_g2_point(input: &[u8]) -> Result<G2Affine, ExitCode> {
    let ba = read_fq2(&input[0..FQ2_SIZE])?;
    let bb = read_fq2(&input[FQ2_SIZE..2 * FQ2_SIZE])?;
    new_g2_point(ba, bb)
}

/// Creates a new `G2` point from the given Fq2 coordinates
#[inline]
pub fn new_g2_point(x: Fq2, y: Fq2) -> Result<G2Affine, ExitCode> {
    let point = if x.is_zero() && y.is_zero() {
        G2Affine::zero()
    } else {
        let point = G2Affine::new_unchecked(x, y);
        if !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve() {
            return Err(ExitCode::PrecompileError);
        }
        point
    };
    Ok(point)
}

/// Encodes a G1 point into a byte array.
///
/// Converts a G1 point in Jacobian coordinates to affine coordinates and
/// serializes the x and y coordinates as big-endian byte arrays.
///
/// Note: If the point is the point at infinity, this function returns
/// all zeroes.
#[inline]
pub fn encode_g1_point(point: G1Affine) -> [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
    let mut output = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
    let Some((x, y)) = point.xy() else {
        return output;
    };

    let mut x_bytes = [0u8; FQ_SIZE];
    if x.serialize_uncompressed(&mut x_bytes[..]).is_err() {
        return output; // Return zero-filled output on serialization failure
    }

    let mut y_bytes = [0u8; FQ_SIZE];
    if y.serialize_uncompressed(&mut y_bytes[..]).is_err() {
        return output; // Return zero-filled output on serialization failure
    }

    // Convert to big endian by reversing the bytes.
    x_bytes.reverse();
    y_bytes.reverse();

    // Place x in the first half, y in the second half.
    output[0..FQ_SIZE].copy_from_slice(&x_bytes);
    output[FQ_SIZE..(FQ_SIZE * 2)].copy_from_slice(&y_bytes);

    output
}

// BN254 helper functions for compression/decompression operations

/// Parse BN254 G1 point from decompressed bytes
pub fn g1_from_decompressed_bytes(
    bytes: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
) -> Result<G1Affine, ()> {
    if *bytes == [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
        return Ok(G1Affine::zero());
    }
    let reader = &bytes[..];
    let affine = G1Affine::deserialize_with_mode(reader, Compress::No, Validate::Yes);

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

/// Parse BN254 G1 point from compressed bytes
pub fn g1_from_compressed_bytes(
    bytes: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
) -> Result<G1Affine, ()> {
    if *bytes == [0u8; BN254_G1_POINT_COMPRESSED_SIZE] {
        return Ok(G1Affine::zero());
    }
    let reader = &bytes[..];
    let affine = G1Affine::deserialize_with_mode(reader, Compress::Yes, Validate::Yes);

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

/// Parse BN254 G2 point from decompressed bytes
pub fn g2_from_decompressed_bytes(
    bytes: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
) -> Result<G2Affine, ()> {
    if *bytes == [0u8; BN254_G2_POINT_DECOMPRESSED_SIZE] {
        return Ok(G2Affine::zero());
    }
    let reader = &bytes[..];
    let affine = G2Affine::deserialize_with_mode(reader, Compress::No, Validate::Yes);

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

/// Parse BN254 G2 point from compressed bytes
pub fn g2_from_compressed_bytes(
    bytes: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
) -> Result<G2Affine, ()> {
    if *bytes == [0u8; BN254_G2_POINT_COMPRESSED_SIZE] {
        return Ok(G2Affine::zero());
    }
    let reader = &bytes[..];
    let affine = G2Affine::deserialize_with_mode(reader, Compress::Yes, Validate::Yes);

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

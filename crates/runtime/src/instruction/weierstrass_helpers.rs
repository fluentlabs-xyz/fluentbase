use ark_ec::{pairing::Pairing, AffineRepr};
use ark_serialize::{CanonicalDeserialize, Compress, Validate};

pub fn convert_endianness<const CHUNK_SIZE: usize, const ARRAY_SIZE: usize>(
    bytes: &[u8; ARRAY_SIZE],
) -> [u8; ARRAY_SIZE] {
    let reversed: [_; ARRAY_SIZE] = bytes
        .chunks_exact(CHUNK_SIZE)
        .flat_map(|chunk| chunk.iter().rev().copied())
        .enumerate()
        .fold([0u8; ARRAY_SIZE], |mut acc, (i, v)| {
            acc[i] = v;
            acc
        });
    reversed
}

pub const G1_POINT_COMPRESSED_SIZE: usize = 32;
pub const G1_POINT_UNCOMPRESSED_SIZE: usize = 64;
pub const G2_POINT_COMPRESSED_SIZE: usize = 64;
pub const G2_POINT_UNCOMPRESSED_SIZE: usize = 128;
pub const PAIRING_ELEMENT_UNCOMPRESSED_LEN: usize =
    G1_POINT_UNCOMPRESSED_SIZE + G2_POINT_UNCOMPRESSED_SIZE;

type G1 = ark_bn254::g1::G1Affine;
type G2 = ark_bn254::g2::G2Affine;

pub fn g1_from_decompressed_bytes(bytes: &[u8; G1_POINT_UNCOMPRESSED_SIZE]) -> Result<G1, ()> {
    if *bytes == [0u8; G1_POINT_UNCOMPRESSED_SIZE] {
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

pub fn g1_from_compressed_bytes(bytes: &[u8; G1_POINT_COMPRESSED_SIZE]) -> Result<G1, ()> {
    if *bytes == [0u8; G1_POINT_COMPRESSED_SIZE] {
        return Ok(G1::zero());
    }
    // let bytes =
    //     convert_endianness::<G1_POINT_COMPRESSED_SIZE, G1_POINT_COMPRESSED_SIZE>(bytes);
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

pub fn g2_from_decompressed_bytes(bytes: &[u8; G2_POINT_UNCOMPRESSED_SIZE]) -> Result<G2, ()> {
    if *bytes == [0u8; G2_POINT_UNCOMPRESSED_SIZE] {
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

pub fn g2_from_compressed_bytes(bytes: &[u8; G2_POINT_COMPRESSED_SIZE]) -> Result<G2, ()> {
    if *bytes == [0u8; G2_POINT_COMPRESSED_SIZE] {
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

use ark_ec::AffineRepr;
use ark_serialize::{CanonicalDeserialize, Compress, Validate};
use fluentbase_types::{
    BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE,
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

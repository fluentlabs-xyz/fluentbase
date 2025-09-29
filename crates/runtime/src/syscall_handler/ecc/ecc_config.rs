//! General curve configuration system for Weierstrass curves
//!
//! This module provides a comprehensive configuration system for different Weierstrass curves
//! and their operations, following the pattern established in weierstrass_recover.rs.

use fluentbase_types::{
    BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
    CURVE256R1_POINT_COMPRESSED_SIZE, CURVE256R1_POINT_DECOMPRESSED_SIZE,
};
use sp1_curves::{
    weierstrass::{bls12_381::Bls12381, bn254::Bn254, secp256k1::Secp256k1, SwCurve},
    CurveType, EllipticCurve,
};

/// Configuration trait for signature verification operations
pub trait VerifyConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;
    /// Size of the message hash in bytes (typically 32 for SHA-256)
    const MESSAGE_HASH_SIZE: usize;
    /// Size of the signature r component in bytes (typically 32)
    const SIGNATURE_R_SIZE: usize;
    /// Size of the signature s component in bytes (typically 32)
    const SIGNATURE_S_SIZE: usize;
    /// Size of the public key x coordinate in bytes (typically 32)
    const PUBLIC_KEY_X_SIZE: usize;
    /// Size of the public key y coordinate in bytes (typically 32)
    const PUBLIC_KEY_Y_SIZE: usize;
    /// Total input size in bytes (sum of all components above)
    const TOTAL_INPUT_SIZE: usize;
}

/// Configuration trait for public key recovery operations
pub trait RecoverConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;

    /// Size of the signature in bytes (typically 64 for ECDSA)
    const SIGNATURE_SIZE: usize;

    /// Size of the uncompressed public key in bytes (typically 65 for Secp256k1)
    const PUBLIC_KEY_SIZE: usize;

    /// Size of the message digest in bytes (typically 32 for SHA-256)
    const DIGEST_SIZE: usize;
}

/// Configuration trait for point addition operations
pub trait AddConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;

    /// Size of a single point in bytes (input/output)
    const POINT_SIZE: usize;

    /// The elliptic curve implementation type
    type EllipticCurve: EllipticCurve;
}

/// Configuration trait for scalar multiplication operations
pub trait MulConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;
    /// Size of the point in bytes
    const POINT_SIZE: usize;
    /// Size of the scalar in bytes
    const SCALAR_SIZE: usize;
}

// /// Configuration trait for point compression operations
// pub trait CompressConfig {
//     /// The curve type
//     const CURVE_TYPE: CurveType;
//     /// Size of the uncompressed point in bytes
//     const UNCOMPRESSED_SIZE: usize;
//     /// Size of the compressed point in bytes
//     const COMPRESSED_SIZE: usize;
// }

// /// Configuration trait for point decompression operations
// pub trait DecompressConfig {
//     /// The curve type
//     const CURVE_TYPE: CurveType;
//     /// Size of the compressed point in bytes
//     const COMPRESSED_SIZE: usize;
//     /// Size of the uncompressed point in bytes
//     const UNCOMPRESSED_SIZE: usize;
// }

/// Configuration trait for mapping operations (field elements to curve points)
pub trait MapConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;
    /// Size of the input field element in bytes
    const INPUT_SIZE: usize;
    /// Size of the output point in bytes
    const OUTPUT_SIZE: usize;
}

/// Configuration trait for pairing operations
pub trait PairingConfig {
    /// The curve type
    const CURVE_TYPE: CurveType;
    /// Size of G1 point in bytes
    const G1_SIZE: usize;
    /// Size of G2 point in bytes
    const G2_SIZE: usize;
    /// Size of the pairing result in bytes
    const RESULT_SIZE: usize;
}

// ============================================================================
// Secp256k1 Configurations
// ============================================================================

/// Secp256k1 signature verification configuration
pub struct Secp256k1VerifyConfig;
impl VerifyConfig for Secp256k1VerifyConfig {
    const CURVE_TYPE: CurveType = CurveType::Secp256k1;
    const MESSAGE_HASH_SIZE: usize = 32;
    const SIGNATURE_R_SIZE: usize = 32;
    const SIGNATURE_S_SIZE: usize = 32;
    const PUBLIC_KEY_X_SIZE: usize = 32;
    const PUBLIC_KEY_Y_SIZE: usize = 32;
    const TOTAL_INPUT_SIZE: usize = 160; // 32 + 32 + 32 + 32 + 32
}

/// Secp256k1 public key recovery configuration
pub struct Secp256k1RecoverConfig;
impl RecoverConfig for Secp256k1RecoverConfig {
    const CURVE_TYPE: CurveType = CurveType::Secp256k1;
    const SIGNATURE_SIZE: usize = 64;
    const PUBLIC_KEY_SIZE: usize = 65;
    const DIGEST_SIZE: usize = 32;
}

/// Secp256k1 point addition configuration
pub struct Secp256k1AddConfig;
impl AddConfig for Secp256k1AddConfig {
    const CURVE_TYPE: CurveType = CurveType::Secp256k1;
    const POINT_SIZE: usize = 65; // uncompressed
    type EllipticCurve = SwCurve<Secp256k1>;
}

// ============================================================================
// Secp256r1 (P-256) Configurations
// ============================================================================

/// Secp256r1 signature verification configuration
pub struct Secp256r1VerifyConfig;
impl VerifyConfig for Secp256r1VerifyConfig {
    const CURVE_TYPE: CurveType = CurveType::Secp256r1;
    const MESSAGE_HASH_SIZE: usize = 32;
    const SIGNATURE_R_SIZE: usize = 32;
    const SIGNATURE_S_SIZE: usize = 32;
    const PUBLIC_KEY_X_SIZE: usize = 32;
    const PUBLIC_KEY_Y_SIZE: usize = 32;
    const TOTAL_INPUT_SIZE: usize = 160; // 32 + 32 + 32 + 32 + 32
}

// ============================================================================
// BLS12-381 Configurations
// ============================================================================

/// BLS12-381 G1 point addition configuration
pub struct Bls12381G1AddConfig;
impl AddConfig for Bls12381G1AddConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const POINT_SIZE: usize = 96; // G1_UNCOMPRESSED_SIZE
    type EllipticCurve = SwCurve<Bls12381>;
}

/// BLS12-381 G2 point addition configuration
pub struct Bls12381G2AddConfig;
impl AddConfig for Bls12381G2AddConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const POINT_SIZE: usize = 192; // G2_UNCOMPRESSED_SIZE
    type EllipticCurve = SwCurve<Bls12381>;
}

/// BLS12-381 G1 scalar multiplication configuration
pub struct Bls12381G1MulConfig;
impl MulConfig for Bls12381G1MulConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const POINT_SIZE: usize = 96; // G1_UNCOMPRESSED_SIZE
    const SCALAR_SIZE: usize = 32;
}

/// BLS12-381 G2 scalar multiplication configuration
pub struct Bls12381G2MulConfig;
impl MulConfig for Bls12381G2MulConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const POINT_SIZE: usize = 192; // G2_UNCOMPRESSED_SIZE
    const SCALAR_SIZE: usize = 32;
}

/// BLS12-381 G1 mapping configuration (Fp -> G1)
pub struct Bls12381G1MapConfig;
impl MapConfig for Bls12381G1MapConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const INPUT_SIZE: usize = 64; // PADDED_FP_SIZE
    const OUTPUT_SIZE: usize = 96; // G1_UNCOMPRESSED_SIZE
}

/// BLS12-381 G2 mapping configuration (Fp2 -> G2)
pub struct Bls12381G2MapConfig;
impl MapConfig for Bls12381G2MapConfig {
    const CURVE_TYPE: CurveType = CurveType::Bls12381;
    const INPUT_SIZE: usize = 128; // PADDED_FP2_SIZE
    const OUTPUT_SIZE: usize = 192; // G2_UNCOMPRESSED_SIZE
}

// ============================================================================
// BN254 Configurations
// ============================================================================

/// BN254 G1 point addition configuration
pub struct Bn254G1AddConfig;
impl AddConfig for Bn254G1AddConfig {
    const CURVE_TYPE: CurveType = CurveType::Bn254;
    const POINT_SIZE: usize = 64; // uncompressed
    type EllipticCurve = SwCurve<Bn254>;
}

/// BN254 G1 scalar multiplication configuration
pub struct Bn254G1MulConfig;
impl MulConfig for Bn254G1MulConfig {
    const CURVE_TYPE: CurveType = CurveType::Bn254;
    const POINT_SIZE: usize = 64; // uncompressed
    const SCALAR_SIZE: usize = 32;
}

/// BN254 G2 scalar multiplication configuration
pub struct Bn254G2MulConfig;
impl MulConfig for Bn254G2MulConfig {
    const CURVE_TYPE: CurveType = CurveType::Bn254;
    const POINT_SIZE: usize = 128; // uncompressed
    const SCALAR_SIZE: usize = 32;
}

/// BN254 pairing configuration
pub struct Bn254PairingConfig;
impl PairingConfig for Bn254PairingConfig {
    const CURVE_TYPE: CurveType = CurveType::Bn254;
    const G1_SIZE: usize = 64; // uncompressed
    const G2_SIZE: usize = 128; // uncompressed
    const RESULT_SIZE: usize = 32; // Fp12 compressed
}

pub enum Curve {
    G1,
    G2,
}

pub enum Mode {
    Compress,
    Decompress,
}

/// Generic trait for curve-specific compress/decompress operations
pub trait CurveConfig {
    const CURVE: Curve;
    const MODE: Mode;
    const CURVE_TYPE: CurveType;

    fn input_point_len() -> usize;
    fn output_point_len() -> usize;
}

/// BN254 G1 Compress configuration
pub struct Bn254G1CompressConfig;
impl CurveConfig for Bn254G1CompressConfig {
    const CURVE: Curve = Curve::G1;
    const MODE: Mode = Mode::Compress;
    const CURVE_TYPE: CurveType = CurveType::Bn254;

    fn input_point_len() -> usize {
        BN254_G1_POINT_DECOMPRESSED_SIZE
    }

    fn output_point_len() -> usize {
        BN254_G1_POINT_COMPRESSED_SIZE
    }
}

/// BN254 G1 Decompress configuration
pub struct Bn254G1DecompressConfig;
impl CurveConfig for Bn254G1DecompressConfig {
    const CURVE: Curve = Curve::G1;
    const MODE: Mode = Mode::Decompress;
    const CURVE_TYPE: CurveType = CurveType::Bn254;

    fn input_point_len() -> usize {
        BN254_G1_POINT_COMPRESSED_SIZE
    }

    fn output_point_len() -> usize {
        BN254_G1_POINT_DECOMPRESSED_SIZE
    }
}

/// BN254 G2 Compress configuration
pub struct Bn254G2CompressConfig;
impl CurveConfig for Bn254G2CompressConfig {
    const CURVE: Curve = Curve::G2;
    const MODE: Mode = Mode::Compress;
    const CURVE_TYPE: CurveType = CurveType::Bn254;

    fn input_point_len() -> usize {
        BN254_G2_POINT_DECOMPRESSED_SIZE
    }

    fn output_point_len() -> usize {
        BN254_G2_POINT_COMPRESSED_SIZE
    }
}

/// BN254 G2 Decompress configuration
pub struct Bn254G2DecompressConfig;
impl CurveConfig for Bn254G2DecompressConfig {
    const CURVE: Curve = Curve::G2;
    const MODE: Mode = Mode::Decompress;
    const CURVE_TYPE: CurveType = CurveType::Bn254;

    fn input_point_len() -> usize {
        BN254_G2_POINT_COMPRESSED_SIZE
    }

    fn output_point_len() -> usize {
        BN254_G2_POINT_DECOMPRESSED_SIZE
    }
}

/// Secp256k1 Decompress configuration
pub struct Secp256k1DecompressConfig;
impl CurveConfig for Secp256k1DecompressConfig {
    const CURVE: Curve = Curve::G1;
    const MODE: Mode = Mode::Decompress;
    const CURVE_TYPE: CurveType = CurveType::Secp256k1;

    fn input_point_len() -> usize {
        CURVE256R1_POINT_COMPRESSED_SIZE
    }

    fn output_point_len() -> usize {
        CURVE256R1_POINT_DECOMPRESSED_SIZE
    }
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

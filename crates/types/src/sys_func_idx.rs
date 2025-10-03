use strum_macros::{Display, FromRepr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display, FromRepr)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum SysFuncIdx {
    // input/output & state control (0x00)
    EXIT = 0x0001,
    STATE = 0x0002,
    READ_INPUT = 0x0003,
    INPUT_SIZE = 0x0004,
    WRITE_OUTPUT = 0x0005,
    OUTPUT_SIZE = 0x0006,
    READ_OUTPUT = 0x0007,
    EXEC = 0x0009,
    RESUME = 0x000a,
    FORWARD_OUTPUT = 0x000b,
    CHARGE_FUEL_MANUALLY = 0x000c,
    FUEL = 0x000d,
    // PREIMAGE_SIZE = 0x000e,
    // PREIMAGE_COPY = 0x000f,
    DEBUG_LOG = 0x0010,
    CHARGE_FUEL = 0x0011,
    ENTER_UNCONSTRAINED = 0x0012,
    EXIT_UNCONSTRAINED = 0x0013,
    WRITE_FD = 0x0014,

    // hashing functions (0x01)
    // #[deprecated(note = "use permute instead")]
    KECCAK256 = 0x0101,
    KECCAK256_PERMUTE = 0x0102,
    POSEIDON = 0x0103,
    // POSEIDON_HASH = 0x0104,
    SHA256_EXTEND = 0x0105,
    SHA256_COMPRESS = 0x0106,
    // #[deprecated(note = "use extend/compress instead")]
    SHA256 = 0x0118,
    BLAKE3 = 0x0117,

    // ed25519 (0x02)
    ED25519_DECOMPRESS = 0x0201,
    ED25519_ADD = 0x0202,

    // fp1/fp2 tower field (0x03)
    TOWER_FP1_BN254_ADD = 0x0301,
    TOWER_FP1_BN254_SUB = 0x0302,
    TOWER_FP1_BN254_MUL = 0x0303,
    TOWER_FP1_BLS12381_ADD = 0x0304,
    TOWER_FP1_BLS12381_SUB = 0x0305,
    TOWER_FP1_BLS12381_MUL = 0x0306,
    TOWER_FP2_BN254_ADD = 0x0307,
    TOWER_FP2_BN254_SUB = 0x0308,
    TOWER_FP2_BN254_MUL = 0x0309,
    TOWER_FP2_BLS12381_ADD = 0x030a,
    TOWER_FP2_BLS12381_SUB = 0x030b,
    TOWER_FP2_BLS12381_MUL = 0x030c,

    // secp256k1 (0x04)
    // SECP256K1_RECOVER = 0x00401,
    SECP256K1_ADD = 0x0402,
    SECP256K1_DECOMPRESS = 0x0403,
    SECP256K1_DOUBLE = 0x0404,

    // secp256r1 (0x05)
    // SECP256R1_VERIFY = 0x0501,

    // bls12381 (0x06)
    BLS12381_G1_ADD = 0x0601,
    BLS12381_G1_MSM = 0x0602,
    BLS12381_G2_ADD = 0x0603,
    BLS12381_G2_MSM = 0x0604,
    BLS12381_PAIRING = 0x0605,
    BLS12381_MAP_G1 = 0x0606,
    BLS12381_MAP_G2 = 0x0607,

    // bn254 (0x07)
    BN254_ADD = 0x0701,
    BN254_DOUBLE = 0x0702,
    BN254_MUL = 0x0703,
    BN254_MULTI_PAIRING = 0x0704,
    BN254_G1_COMPRESS = 0x0705,
    BN254_G1_DECOMPRESS = 0x0706,
    BN254_G2_COMPRESS = 0x0707,
    BN254_G2_DECOMPRESS = 0x0708,

    // uint256 (0x08)
    UINT256_MUL_MOD = 0x0801,
    UINT256_X2048_MUL = 0x0802,
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}

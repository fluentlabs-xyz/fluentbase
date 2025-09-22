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
    PREIMAGE_SIZE = 0x000e,
    PREIMAGE_COPY = 0x000f,
    DEBUG_LOG = 0x0010,
    CHARGE_FUEL = 0x0011,

    // hashing functions (0x01)
    #[deprecated(note = "use permute instead")]
    KECCAK256 = 0x0101,
    KECCAK256_PERMUTE = 0x0102,
    POSEIDON = 0x0103,
    // POSEIDON_HASH = 0x0104,
    SHA256_EXTEND = 0x0105,
    SHA256_COMPRESS = 0x0106,
    #[deprecated(note = "use extend/compress instead")]
    SHA256 = 0x0118,
    BLAKE3 = 0x0117,

    // ed25519 (0x02)
    ED25519_DECOMPRESS = 0x0201,
    ED25519_ADD = 0x0202,
    ED25519_SUB = 0x0203,
    ED25519_MULTISCALAR_MUL = 0x0114,
    ED25519_MUL = 0x0205,

    // ristretto255 (0x03)
    RISTRETTO255_DECOMPRESS = 0x0301,
    RISTRETTO255_ADD = 0x0302,
    RISTRETTO255_SUB = 0x0303,
    RISTRETTO255_MULTISCALAR_MUL = 0x0304,
    RISTRETTO255_MUL = 0x0305,

    // secp256k1 (0x04)
    SECP256K1_RECOVER = 0x00401,
    SECP256K1_ADD = 0x0402,
    SECP256K1_DECOMPRESS = 0x0403,
    SECP256K1_DOUBLE = 0x0404,

    // secp256r1 (0x05)
    SECP256R1_VERIFY = 0x0501,

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
    BN254_FP_ADD = 0x0709,
    BN254_FP_SUB = 0x070a,
    BN254_FP_MUL = 0x070b,
    BN254_FP2_ADD = 0x070c,
    BN254_FP2_SUB = 0x070d,
    BN254_FP2_MUL = 0x070e,

    // uint256 (0x08)
    BIGINT_UINT256_MUL = 0x0801,
    BIGINT_MOD_EXP = 0x0802,

    // sp1 (0x51)
    WRITE_FD = 0x5101,
    ENTER_UNCONSTRAINED = 0x5102,
    EXIT_UNCONSTRAINED = 0x5103,
    COMMIT = 0x5104,
    COMMIT_DEFERRED_PROOFS = 0x5105,
    VERIFY_SP1_PROOF = 0x5106,
    HINT_LEN = 0x5107,
    HINT_READ = 0x5108,
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}

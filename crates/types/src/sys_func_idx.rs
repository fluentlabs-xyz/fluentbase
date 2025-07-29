use strum_macros::{Display, FromRepr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display, FromRepr)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum SysFuncIdx {
    // SYS host
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

    // hashing
    KECCAK256 = 0x0101,
    KECCAK256_PERMUTE = 0x0102,
    // POSEIDON = 0x0103,
    // POSEIDON_HASH = 0x0104,
    SHA256_EXTEND = 0x0105,
    SHA256_COMPRESS = 0x0106,

    // ed25519
    ED25519_ADD = 0x0107,
    ED25519_DECOMPRESS = 0x0108,

    // secp256k1
    SECP256K1_RECOVER = 0x0110,
    SECP256K1_ADD = 0x0111,
    SECP256K1_DECOMPRESS = 0x0112,
    SECP256K1_DOUBLE = 0x0113,

    // bls12381
    BLS12381_DECOMPRESS = 0x0120,
    BLS12381_ADD = 0x0121,
    BLS12381_DOUBLE = 0x0122,
    BLS12381_FP_ADD = 0x0123,
    BLS12381_FP_SUB = 0x0124,
    BLS12381_FP_MUL = 0x0125,
    BLS12381_FP2_ADD = 0x0126,
    BLS12381_FP2_SUB = 0x0127,
    BLS12381_FP2_MUL = 0x0128,

    // bn254
    BN254_ADD = 0x0130,
    BN254_DOUBLE = 0x0131,
    BN254_MUL = 0x0138,
    BN254_FP_ADD = 0x0132,
    BN254_FP_SUB = 0x0133,
    BN254_FP_MUL = 0x0134,
    BN254_FP2_ADD = 0x0135,
    BN254_FP2_SUB = 0x0136,
    BN254_FP2_MUL = 0x0137,

    // uint256
    UINT256_MUL = 0x011D,
    // sp1
    // WRITE_FD = 0x0202,
    // ENTER_UNCONSTRAINED = 0x0203,
    // EXIT_UNCONSTRAINED = 0x0204,
    // COMMIT = 0x0210,
    // COMMIT_DEFERRED_PROOFS = 0x021A,
    // VERIFY_SP1_PROOF = 0x021B,
    // HINT_LEN = 0x02F0,
    // HINT_READ = 0x02F1,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::EXIT => 1,
            SysFuncIdx::STATE => 1,
            SysFuncIdx::READ_INPUT => 1,
            SysFuncIdx::INPUT_SIZE => 1,
            SysFuncIdx::WRITE_OUTPUT => 1,
            SysFuncIdx::KECCAK256 => 1,
            SysFuncIdx::SECP256K1_RECOVER => 1,
            _ => 1, //unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}

use crate::{SysFuncIdx, FUEL_DENOM_RATE};
use rwasm::{instruction_set, InstructionSet, TrapCode};

/// The maximum allowed value for the `x` parameter used in linear gas cost calculation
/// of builtins.
/// This limit ensures the result does not overflow a `u32`.
/// Specifically:
/// - `word_cost` is assumed to be at most `8191` (i.e. `2^13-1`)
/// - `base_cost` is assumed to be at most `2_147_483_647` (i.e. `2^31-1`)
const FUEL_MAX_LINEAR_X: u32 = 262_143; // 2^18 - 1

/// In this file, we define the fuel procedures that will be inserted by the rwasm translator
/// before the builtin calls. Each fuel procedure is a set of rwasm Opcodes that will be
/// executed. Fuel procedures can potentially access the variables of the function they are
/// inserted into.

/// Formula: cost = base_cost + word_cost * (x + 31) / 32, where x is one of the parameters
/// of the builtin. What parameter is used depends on the builtin and defined by local_depth.
///
/// Local depth equal to 1 means the last parameter, increase by 1 for each previous parameter. For
/// more info on that, checkout implementation of local gets a visitor in the rwasm codebase (fn
/// visit_local_get).
///
/// Word is defined as 32 bytes, the same as in the EVM.
macro_rules! linear_fuel {
    ($local_depth:expr, $base_cost:expr, $word_cost:expr) => {{
        // compile-time overflow check
        const _: () = {
            assert!(
                ($base_cost as u128) + ($word_cost as u128) * (FUEL_MAX_LINEAR_X as u128) <= (u32::MAX as u128),
                "base_cost + word_cost * FUEL_MAX_LINEAR_X must fit into u32"
            );
        };

        instruction_set! {
            LocalGet($local_depth) // Compare x with FUEL_MAX_LINEAR_X
            I32Const(FUEL_MAX_LINEAR_X)
            I32GtU
            BrIfEqz(2)
            Trap(TrapCode::IntegerOverflow)

            LocalGet($local_depth) // Calculate cost
            I32Const(31)
            I32Add
            I32Const(32)
            I32DivU
            I32Const($word_cost)
            I32Mul
            I32Const($base_cost)
            I32Add
            I32Const(0) // Push two 32-bit values, which are interpreted as a single 64-bit value inside the builtin.
            Call(SysFuncIdx::CHARGE_FUEL)
        }
    }};
}

macro_rules! const_fuel {
    ($cost:expr) => {
        instruction_set! {
            I32Const($cost)
            I32Const(0) // Push two 32-bit values, which are interpreted as a single 64-bit value inside the builtin.
            Call(SysFuncIdx::CHARGE_FUEL)
        }
    };
}

macro_rules! no_fuel {
    () => {
        instruction_set! {}
    };
}

// Common fuel cost constants
pub const LOW_FUEL_COST: u32 = 1 * FUEL_DENOM_RATE as u32;
pub const COPY_BASE_FUEL_COST: u32 = 20 * FUEL_DENOM_RATE as u32;
pub const COPY_WORD_FUEL_COST: u32 = 3 * FUEL_DENOM_RATE as u32;
pub const DEBUG_LOG_BASE_FUEL_COST: u32 = 50 * FUEL_DENOM_RATE as u32;
pub const DEBUG_LOG_WORD_FUEL_COST: u32 = 16 * FUEL_DENOM_RATE as u32;
pub const CHARGE_FUEL_BASE_COST: u32 = 20 * FUEL_DENOM_RATE as u32;
pub const KECCAK_BASE_FUEL_COST: u32 = 30 * FUEL_DENOM_RATE as u32;
pub const KECCAK_WORD_FUEL_COST: u32 = 6 * FUEL_DENOM_RATE as u32;
pub const SHA256_BASE_FUEL_COST: u32 = 60 * FUEL_DENOM_RATE as u32;
pub const SHA256_WORD_FUEL_COST: u32 = 12 * FUEL_DENOM_RATE as u32;
pub const BLAKE3_BASE_FUEL_COST: u32 = 60 * FUEL_DENOM_RATE as u32;
pub const BLAKE3_WORD_FUEL_COST: u32 = 12 * FUEL_DENOM_RATE as u32;

// Ed25519
pub const ED25519_DECOMPRESS_COST: u32 = 5_000 * FUEL_DENOM_RATE as u32;
pub const ED25519_ADD_COST: u32 = 1_500 * FUEL_DENOM_RATE as u32;

// Tower field (BN254/BLS12-381)
pub const FP1_ADD_COST: u32 = 6 * FUEL_DENOM_RATE as u32;
pub const FP1_MUL_COST: u32 = 24 * FUEL_DENOM_RATE as u32;
pub const FP2_ADD_COST: u32 = 12 * FUEL_DENOM_RATE as u32;
pub const FP2_MUL_COST: u32 = 48 * FUEL_DENOM_RATE as u32;
pub const FP1_BLS_ADD_COST: u32 = 8 * FUEL_DENOM_RATE as u32;
pub const FP1_BLS_MUL_COST: u32 = 32 * FUEL_DENOM_RATE as u32;
pub const FP2_BLS_ADD_COST: u32 = 16 * FUEL_DENOM_RATE as u32;
pub const FP2_BLS_MUL_COST: u32 = 64 * FUEL_DENOM_RATE as u32;

// Secp256k1
pub const SECP256K1_ADD_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const SECP256K1_DOUBLE_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const SECP256K1_DECOMPRESS_COST: u32 = 4_000 * FUEL_DENOM_RATE as u32;

// BN254
pub const BN254_ADD_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const BN254_DOUBLE_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const BN254_MUL_COST: u32 = 6_000 * FUEL_DENOM_RATE as u32;
pub const BN254_PAIRING_COST: u32 = 45_000 * FUEL_DENOM_RATE as u32;
pub const BN254_G1_COMPRESS_COST: u32 = 600 * FUEL_DENOM_RATE as u32;
pub const BN254_G1_DECOMPRESS_COST: u32 = 600 * FUEL_DENOM_RATE as u32;
pub const BN254_G2_COMPRESS_COST: u32 = 1_200 * FUEL_DENOM_RATE as u32;
pub const BN254_G2_DECOMPRESS_COST: u32 = 1_200 * FUEL_DENOM_RATE as u32;

// BLS12-381
pub const BLS_G1_ADD_COST: u32 = 600 * FUEL_DENOM_RATE as u32;
pub const BLS_G2_ADD_COST: u32 = 4_500 * FUEL_DENOM_RATE as u32;
pub const BLS_PAIRING_COST: u32 = 45_000 * FUEL_DENOM_RATE as u32;
pub const BLS_MAP_G1_COST: u32 = 8_000 * FUEL_DENOM_RATE as u32;
pub const BLS_MAP_G2_COST: u32 = 80_000 * FUEL_DENOM_RATE as u32;

// Big integer
pub const UINT256_MUL_MOD_COST: u32 = 8 * FUEL_DENOM_RATE as u32;
pub const UINT256_X2048_MUL_COST: u32 = 5_000 * FUEL_DENOM_RATE as u32;

pub(crate) fn emit_fuel_procedure(sys_func_idx: SysFuncIdx) -> InstructionSet {
    use SysFuncIdx::*;
    match sys_func_idx {
        // input/output & state control (0x00)
        EXIT => no_fuel!(),
        STATE => const_fuel!(LOW_FUEL_COST),
        READ_INPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        INPUT_SIZE => const_fuel!(LOW_FUEL_COST),
        WRITE_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        OUTPUT_SIZE => const_fuel!(LOW_FUEL_COST),
        READ_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        EXEC => no_fuel!(),
        RESUME => no_fuel!(),
        FORWARD_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        CHARGE_FUEL_MANUALLY => no_fuel!(),
        FUEL => const_fuel!(LOW_FUEL_COST),
        DEBUG_LOG => linear_fuel!(1, DEBUG_LOG_BASE_FUEL_COST, DEBUG_LOG_WORD_FUEL_COST),
        CHARGE_FUEL => const_fuel!(CHARGE_FUEL_BASE_COST),

        // hashing functions (0x01)
        KECCAK256 => linear_fuel!(2, KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST),
        KECCAK256_PERMUTE => const_fuel!(KECCAK_BASE_FUEL_COST),
        POSEIDON => linear_fuel!(2, 100, 20),
        SHA256_EXTEND => const_fuel!(SHA256_BASE_FUEL_COST),
        SHA256_COMPRESS => const_fuel!(SHA256_BASE_FUEL_COST),
        SHA256 => linear_fuel!(2, SHA256_BASE_FUEL_COST, SHA256_WORD_FUEL_COST),
        BLAKE3 => linear_fuel!(2, BLAKE3_BASE_FUEL_COST, BLAKE3_WORD_FUEL_COST),

        // ed25519 (0x02)
        ED25519_DECOMPRESS => const_fuel!(ED25519_DECOMPRESS_COST),
        ED25519_ADD => const_fuel!(ED25519_ADD_COST),

        // fp1/fp2 tower field (0x03)
        TOWER_FP1_BN254_ADD | TOWER_FP1_BN254_SUB => const_fuel!(FP1_ADD_COST),
        TOWER_FP1_BN254_MUL => const_fuel!(FP1_MUL_COST),
        TOWER_FP1_BLS12381_ADD | TOWER_FP1_BLS12381_SUB => const_fuel!(FP1_BLS_ADD_COST),
        TOWER_FP1_BLS12381_MUL => const_fuel!(FP1_BLS_MUL_COST),
        TOWER_FP2_BN254_ADD | TOWER_FP2_BN254_SUB => const_fuel!(FP2_ADD_COST),
        TOWER_FP2_BN254_MUL => const_fuel!(FP2_MUL_COST),
        TOWER_FP2_BLS12381_ADD | TOWER_FP2_BLS12381_SUB => const_fuel!(FP2_BLS_ADD_COST),
        TOWER_FP2_BLS12381_MUL => const_fuel!(FP2_BLS_MUL_COST),

        // secp256k1 (0x04)
        SECP256K1_ADD => const_fuel!(SECP256K1_ADD_COST),
        SECP256K1_DECOMPRESS => const_fuel!(SECP256K1_DECOMPRESS_COST),
        SECP256K1_DOUBLE => const_fuel!(SECP256K1_DOUBLE_COST),

        // bls12381 (0x06)
        BLS12381_ADD => const_fuel!(BLS_G1_ADD_COST),
        // BLS12381_G2_ADD => const_fuel!(BLS_G2_ADD_COST),
        // BLS12381_PAIRING => const_fuel!(BLS_PAIRING_COST),
        // BLS12381_MAP_G1 => const_fuel!(BLS_MAP_G1_COST),
        // BLS12381_MAP_G2 => const_fuel!(BLS_MAP_G2_COST),

        // bn254 (0x07)
        BN254_ADD => const_fuel!(BN254_ADD_COST),
        BN254_DOUBLE => const_fuel!(BN254_DOUBLE_COST),

        // uint256 (0x08)
        UINT256_MUL_MOD => const_fuel!(UINT256_MUL_MOD_COST),
        UINT256_X2048_MUL => const_fuel!(UINT256_X2048_MUL_COST),

        _ => no_fuel!(),
    }
}

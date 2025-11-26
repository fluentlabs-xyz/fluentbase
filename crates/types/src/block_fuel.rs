use crate::SysFuncIdx::{ENTER_UNCONSTRAINED, EXIT_UNCONSTRAINED, WRITE_FD};
use crate::{SysFuncIdx, FUEL_DENOM_RATE};
use rwasm::{SyscallFuelParams, QuadraticFuelParams, LinearFuelParams};

/// The maximum allowed value for the `x` parameter used in linear gas cost calculation
/// of builtins.
/// This limit ensures:
/// 1. Runtime: all intermediate i32 operations do not overflow (WASM constraint)
/// 2. Compile-time: final fuel value fits in u32
///
/// Formula:
/// words = (x + 31) / 32
/// fuel = base_cost + word_cost × words
///
/// Derivation:
/// The bottleneck is I32Mul: words × word_cost ≤ i32::MAX
///
/// words ≈ x / 32, so:
/// (x / 32) × word_cost ≤ 2^31
/// x ≤ (2^31 × 32) / word_cost_max
///
/// Worst case (DEBUG_LOG with FUEL_DENOM_RATE = 20):
/// - word_cost_max = 16 × 20 = 320
///
/// x ≤ (2^31 × 32) / 320 = 214,748,364 bytes (~204 MB)
///
/// We use 128 MB as a safe limit within the theoretical maximum:
const FUEL_MAX_LINEAR_X: u32 = 134_217_728; // 128 MB (2^27)

/// The maximum allowed value for the `x` parameter used in quadratic gas cost calculation
/// of builtins.
/// This limit ensures:
/// 1. Runtime: words × words does not overflow i32 (WASM constraint)
/// 2. Compile-time: final fuel value fits in u32
///
/// Formula:
/// words = (x + 31) / 32
/// fuel = (word_cost × words + words² / divisor) × FUEL_DENOM_RATE
///
/// Derivation:
/// words × words must not overflow i32:
/// words² ≤ i32::MAX (2,147,483,647)
/// words ≤ 46,340
/// x ≤ 46,340 × 32 = 1,482,880 bytes (~1.4 MB)
///
/// We use 1.25 MB as a safe limit within the theoretical maximum:
const FUEL_MAX_QUADRATIC_X: u32 = 1_310_720; // 1.25 MB (2^20 + 2^18)

/// In this file, we define the fuel procedures that will be inserted by the rwasm translator
/// before the builtin calls. Each fuel procedure is a set of rwasm Opcodes that will be
/// executed. Fuel procedures can potentially access the variables of the function they are
/// inserted into.
/// Word is defined as 32 bytes, the same as in the EVM.
macro_rules! no_fuel {
    () => {
        SyscallFuelParams::None
    };
}

macro_rules! const_fuel {
    ($base:expr) => {
        SyscallFuelParams::Const($base as u64)
    };
}

macro_rules! linear_fuel {
    ($param_index:expr, $base:expr, $linear:expr) => {
        SyscallFuelParams::LinearFuel(LinearFuelParams {
            base_fuel: $base as u64,
            param_index: $param_index,
            word_cost: $linear as u64,
        })
    };
}

macro_rules! quadratic_fuel {
    ($local_depth:expr, $word_cost:expr, $divisor:expr) => {
        SyscallFuelParams::QuadraticFuel(QuadraticFuelParams {
            divisor: $divisor as u64,
            param_index: $local_depth,
            word_cost: $word_cost as u64,
        })
    }
}

// Common fuel cost constants
pub const LOW_FUEL_COST: u32 = 20 * FUEL_DENOM_RATE as u32;
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

// Secp256r1
pub const SECP256R1_ADD_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const SECP256R1_DOUBLE_COST: u32 = 150 * FUEL_DENOM_RATE as u32;
pub const SECP256R1_DECOMPRESS_COST: u32 = 4_000 * FUEL_DENOM_RATE as u32;

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
pub const BLS_G1_DOUBLE_COST: u32 = 600 * FUEL_DENOM_RATE as u32;
pub const BLS_G1_DECOMPRESS_COST: u32 = 600 * FUEL_DENOM_RATE as u32;

// Big integer
pub const UINT256_MUL_MOD_COST: u32 = 8 * FUEL_DENOM_RATE as u32;
pub const UINT256_X2048_MUL_COST: u32 = 5_000 * FUEL_DENOM_RATE as u32;

// Quadratic fuel constants (EVM memory expansion formula)
pub const QUADRATIC_WORD_FUEL_COST: u32 = 3;
pub const QUADRATIC_DIVISOR: u32 = 512;

pub(crate) fn calculate_syscall_fuel(sys_func_idx: SysFuncIdx) -> SyscallFuelParams {
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
        EXEC => quadratic_fuel!(3, QUADRATIC_WORD_FUEL_COST, QUADRATIC_DIVISOR),
        RESUME => no_fuel!(),
        FORWARD_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        CHARGE_FUEL_MANUALLY => no_fuel!(),
        FUEL => const_fuel!(LOW_FUEL_COST),
        DEBUG_LOG => linear_fuel!(1, DEBUG_LOG_BASE_FUEL_COST, DEBUG_LOG_WORD_FUEL_COST),
        CHARGE_FUEL => const_fuel!(CHARGE_FUEL_BASE_COST),
        ENTER_UNCONSTRAINED => no_fuel!(),
        EXIT_UNCONSTRAINED => no_fuel!(),
        // TODO: use correct fuel calculations here, once we will implement WRITE_FD
        WRITE_FD => no_fuel!(),

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
        // secp256r1 (0x05)
        SECP256R1_ADD => const_fuel!(SECP256R1_ADD_COST),
        SECP256R1_DECOMPRESS => const_fuel!(SECP256R1_DECOMPRESS_COST),
        SECP256R1_DOUBLE => const_fuel!(SECP256R1_DOUBLE_COST),

        // bls12381 (0x06)
        BLS12381_ADD => const_fuel!(BLS_G1_ADD_COST),
        BLS12381_DECOMPRESS => const_fuel!(BLS_G1_DECOMPRESS_COST),
        BLS12381_DOUBLE => const_fuel!(BLS_G1_DOUBLE_COST),

        // bn254 (0x07)
        BN254_ADD => const_fuel!(BN254_ADD_COST),
        BN254_DOUBLE => const_fuel!(BN254_DOUBLE_COST),

        // uint256 (0x08)
        UINT256_MUL_MOD => const_fuel!(UINT256_MUL_MOD_COST),
        UINT256_X2048_MUL => const_fuel!(UINT256_X2048_MUL_COST),
    }
}

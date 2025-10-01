use crate::SysFuncIdx;
use rwasm::{instruction_set, InstructionSet, TrapCode};

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
                ($base_cost as u128) + ($word_cost as u128) * (MAX_X as u128) <= (u32::MAX as u128),
                "base_cost + word_cost * MAX_X must fit into u32"
            );
        };

        instruction_set! {
            LocalGet($local_depth) // Compare x with MAX_X
            I32Const(MAX_X)
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

/// Values are loosely based on the EVM opcodes in the revm interpreter.
pub const KECCAK_BASE_FUEL_COST: u32 = 30_000;
pub const KECCAK_WORD_FUEL_COST: u32 = 6_000;
pub const COPY_WORD_FUEL_COST: u32 = 3_000;
pub const COPY_BASE_FUEL_COST: u32 = 3_000;
pub const SECP256K1_RECOVER_BASE_FUEL_COST: u32 = 100_000;
pub const LOW_FUEL_COST: u32 = 1_000;
pub const CHARGE_FUEL_BASE_COST: u32 = 1_000;

/// The maximum allowed value for the `x` parameter used in linear gas cost calculation
/// of builtins.
/// This limit ensures the result does not overflow a `u32`.
/// Specifically:
/// - `word_cost` is assumed to be at most `8191` (i.e. `2^13-1`)
/// - `base_cost` is assumed to be at most `2_147_483_647` (i.e. `2^31-1`)
const MAX_X: u32 = 262_143; // 2^18 - 1

pub(crate) fn emit_fuel_procedure(sys_func_idx: SysFuncIdx) -> InstructionSet {
    match sys_func_idx {
        // Builtins charging no fuel (free builtins)
        SysFuncIdx::CHARGE_FUEL_MANUALLY => no_fuel!(),
        SysFuncIdx::EXEC => no_fuel!(),
        SysFuncIdx::EXIT => no_fuel!(),
        SysFuncIdx::RESUME => no_fuel!(),

        // Builtins charging a constant amount of fuel
        SysFuncIdx::CHARGE_FUEL => const_fuel!(CHARGE_FUEL_BASE_COST),
        SysFuncIdx::FUEL => const_fuel!(LOW_FUEL_COST),
        SysFuncIdx::INPUT_SIZE => const_fuel!(LOW_FUEL_COST),
        SysFuncIdx::OUTPUT_SIZE => const_fuel!(LOW_FUEL_COST),
        SysFuncIdx::STATE => const_fuel!(LOW_FUEL_COST),

        // Builtins charging a variable amount of fuel based on a parameter
        SysFuncIdx::DEBUG_LOG => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        SysFuncIdx::FORWARD_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        SysFuncIdx::KECCAK256 => linear_fuel!(2, KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST),
        SysFuncIdx::READ_INPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        SysFuncIdx::READ_OUTPUT => linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST),
        SysFuncIdx::WRITE_OUTPUT => linear_fuel!(1, COPY_WORD_FUEL_COST, COPY_WORD_FUEL_COST),

        _ => no_fuel!(),
    }
}

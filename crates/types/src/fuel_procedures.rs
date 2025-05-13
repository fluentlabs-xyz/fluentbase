/// In this file, we define the fuel procedures that will be inserted by the rwasm translator
/// before the builtin calls. Each fuel procedure is a set of rwasm instructions that will be
/// executed. Fuel procedures can potentially access the variables of the function they are
/// inserted into.
use crate::SysFuncIdx::CHARGE_FUEL;
use rwasm::legacy::{
    core::UntypedValue,
    engine::bytecode::{FuncIdx, Instruction, LocalDepth},
};

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
                ($base_cost as u128) + ($word_cost as u128) * (u32::MAX as u128)
                    <= (u64::MAX as u128),
                "base_cost + word_cost * u32::MAX must fit into u64"
            );
        };

        &[
            Instruction::LocalGet(LocalDepth::from_u32($local_depth)),
            Instruction::I64ExtendI32U, // we extend the length of local variable to u64
            Instruction::I64Const(UntypedValue::from_bits(31)),
            Instruction::I64Add,
            Instruction::I64Const(UntypedValue::from_bits(32)),
            Instruction::I64DivU,
            Instruction::I64Const(UntypedValue::from_bits($word_cost as u64)),
            Instruction::I64Mul,
            Instruction::I64Const(UntypedValue::from_bits($base_cost as u64)),
            Instruction::I64Add,
            Instruction::Call(FuncIdx::from_u32(CHARGE_FUEL as u32)),
        ]
    }};
}

macro_rules! const_fuel {
    ($cost:expr) => {
        &[
            Instruction::I64Const(UntypedValue::from_bits($cost as u64)),
            Instruction::Call(FuncIdx::from_u32(CHARGE_FUEL as u32)),
        ]
    };
}

macro_rules! no_fuel {
    () => {
        &[]
    };
}

/// Constants used to calculate the fuel cost of builtin functions.
/// Their values are loosely based on the EVM opcodes in the revm interpreter.
/// But multiplied by 1000 to compensate denomination.
/// Note: values must be small enough to not overflow u64 when multiplied by u32::MAX
pub const KECCAK_BASE_FUEL_COST: u64 = 30_000; // correspond to KECCAK opcode cost
pub const KECCAK_WORD_FUEL_COST: u64 = 6_000;
pub const COPY_WORD_FUEL_COST: u64 = 3_000; // correspond to COPY opcode cost
pub const COPY_BASE_FUEL_COST: u64 = 3_000;
pub const EXEC_BASE_FUEL_COST: u64 = 2_600_000; // correspond to COLD_ACCOUNT_ACCESS_COST
pub const SECP256K1_RECOVER_BASE_FUEL_COST: u64 = 100_000;
pub const MINIMAL_BASE_FUEL_COST: u64 = 1_000;
pub const CHARGE_FUEL_BASE_COST: u64 = 1_000;

/// This fuel charging procedures will be emitted inside the rwasm translator right before
/// the builtin call.
pub(crate) const KECCAK256_FUEL: &[Instruction] =
    linear_fuel!(2, KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST);
pub(crate) const EXIT_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const STATE_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const INPUT_SIZE_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const OUTPUT_SIZE_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const CHARGE_FUEL_FUEL: &[Instruction] = const_fuel!(CHARGE_FUEL_BASE_COST);
pub(crate) const FUEL_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const READ_INPUT_FUEL: &[Instruction] =
    linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const WRITE_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, COPY_WORD_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const READ_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const EXEC_FUEL: &[Instruction] =
    linear_fuel!(3, EXEC_BASE_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const RESUME_FUEL: &[Instruction] = const_fuel!(EXEC_BASE_FUEL_COST);
pub(crate) const FORWARD_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const CHARGE_FUEL_MANUALLY_FUEL: &[Instruction] = no_fuel!();
pub(crate) const PREIMAGE_SIZE_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const PREIMAGE_COPY_FUEL: &[Instruction] = const_fuel!(MINIMAL_BASE_FUEL_COST);
pub(crate) const DEBUG_LOG_FUEL: &[Instruction] =
    linear_fuel!(1, COPY_BASE_FUEL_COST, COPY_WORD_FUEL_COST);
pub(crate) const SECP256K1_RECOVER_FUEL: &[Instruction] =
    const_fuel!(SECP256K1_RECOVER_BASE_FUEL_COST);

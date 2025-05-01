use crate::SysFuncIdx::CHARGE_FUEL;
use rwasm::{
    core::UntypedValue,
    engine::bytecode::{FuncIdx, Instruction, LocalDepth},
};

/// Formula: cost = base_cost + word_cost * (x + 31) / 32, where x is one of the parameters
/// of the builtin. What parameter is used depends on the builtin and defined by local_depth.
/// Local depth equal to 1 means the last parameter, increase by 1 for each previous parameter.
/// Word is defined as 32 bytes, the same as in the EVM.
macro_rules! linear_fuel {
    ($local_depth:expr, $base_cost:expr, $word_cost:expr) => {
        &[
            Instruction::LocalGet(LocalDepth::from_u32($local_depth)),
            Instruction::I64ExtendI32U,
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
    };
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

pub const KECCAK_BASE_FUEL_COST: u32 = 30; // same value as KECCAK opcode in revm
pub const KECCAK_WORD_FUEL_COST: u32 = 6;
pub const READ_WORD_FUEL_COST: u32 = 3; // same value as COPY opcode in revm
pub const READ_BASE_FUEL_COST: u32 = 3;
pub const WRITE_BASE_FUEL_COST: u32 = 375; // same value as LOG opcode in revm
pub const WRITE_WORD_FUEL_COST: u32 = 8;
pub const EXEC_BASE_FUEL_COST: u32 = 2600; // same value as COLD_ACCOUNT_ACCESS_COST in revm, maybe other value is better
pub const SECP256K1_RECOVER_BASE_FUEL_COST: u32 = 100; // there is no such opcode in revm, but let's set it to 100 for now

/// This fuel charging procedures will be emitted inside the rwasm translator right before
/// the builtin call.
pub(crate) const KECCAK256_FUEL: &[Instruction] =
    linear_fuel!(2, KECCAK_BASE_FUEL_COST, KECCAK_WORD_FUEL_COST);
pub(crate) const EXIT_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const STATE_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const INPUT_SIZE_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const OUTPUT_SIZE_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const CHARGE_FUEL_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const FUEL_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const READ_INPUT_FUEL: &[Instruction] =
    linear_fuel!(1, READ_BASE_FUEL_COST, READ_WORD_FUEL_COST);
pub(crate) const WRITE_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, WRITE_BASE_FUEL_COST, WRITE_WORD_FUEL_COST);
pub(crate) const READ_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, READ_BASE_FUEL_COST, READ_WORD_FUEL_COST);
pub(crate) const EXEC_FUEL: &[Instruction] =
    linear_fuel!(3, EXEC_BASE_FUEL_COST, READ_WORD_FUEL_COST);
pub(crate) const RESUME_FUEL: &[Instruction] = const_fuel!(EXEC_BASE_FUEL_COST);
pub(crate) const FORWARD_OUTPUT_FUEL: &[Instruction] =
    linear_fuel!(1, READ_BASE_FUEL_COST, READ_WORD_FUEL_COST);
pub(crate) const CHARGE_FUEL_MANUALLY_FUEL: &[Instruction] = no_fuel!();
pub(crate) const PREIMAGE_SIZE_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const PREIMAGE_COPY_FUEL: &[Instruction] = const_fuel!(1);
pub(crate) const DEBUG_LOG_FUEL: &[Instruction] =
    linear_fuel!(1, READ_BASE_FUEL_COST, READ_WORD_FUEL_COST);
pub(crate) const SECP256K1_RECOVER_FUEL: &[Instruction] =
    const_fuel!(SECP256K1_RECOVER_BASE_FUEL_COST);

use crate::common::UntypedValue;
use crate::engine::bytecode::{BlockFuel, Instruction, LocalDepth};
use alloc::vec::Vec;

#[derive(Default, Clone)]
pub struct InstructionSet(pub Vec<Instruction>);

macro_rules! impl_opcode {
    ($name:ident, $opcode:ident($into:ident)) => {
        pub fn $name<I: Into<$into>>(&mut self, value: I) {
            self.0.push(Instruction::$opcode(value.into()));
        }
    };
    ($name:ident, $opcode:ident($into:ident, $into2:ident)) => {
        pub fn $name<I: Into<$into>, J: Into<$into2>>(&mut self, value: I, value2: J) {
            self.0.push(Instruction::$opcode(value.into(), value2.into()));
        }
    };
    ($name:ident, $opcode:ident) => {
        pub fn $name(&mut self) {
            self.0.push(Instruction::$opcode);
        }
    };
}

impl From<Vec<Instruction>> for InstructionSet {
    fn from(value: Vec<Instruction>) -> Self {
        Self(value)
    }
}

impl InstructionSet {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, opcode: Instruction) -> u32 {
        let opcode_pos = self.len();
        self.0.push(opcode);
        opcode_pos
    }

    pub fn len(&self) -> u32 {
        self.0.len() as u32
    }

    impl_opcode!(op_unreachable, Unreachable);
    impl_opcode!(op_consume_fuel, ConsumeFuel(BlockFuel));
    impl_opcode!(op_drop, Drop);
    impl_opcode!(op_select, Select);
    impl_opcode!(op_local_get, LocalGet(LocalDepth));
    impl_opcode!(op_local_set, LocalSet(LocalDepth));
    impl_opcode!(op_local_tee, LocalTee(LocalDepth));
    // impl_opcode!(op_br, Br(BranchParams));
    // impl_opcode!(op_br_if_eqz, BrIfEqz(BranchParams));
    // impl_opcode!(op_br_if_nez, BrIfNez(BranchParams));
    // impl_opcode!(op_br_table, BrTable(Index));
    // impl_opcode!(op_return, Return(DropKeep));
    // impl_opcode!(op_return_call_indirect, ReturnCallIndirect(Index, DropKeep));
    // impl_opcode!(op_call, Call(Index));
    // impl_opcode!(op_call_indirect, CallIndirect(Index));
    // impl_opcode!(op_global_get, GlobalGet(Index));
    // impl_opcode!(op_global_set, GlobalSet(Index));
    // add more opcodes
    impl_opcode!(op_i32_const, I32Const(UntypedValue));
    impl_opcode!(op_i64_const, I64Const(UntypedValue));

    pub fn extend<I: Into<InstructionSet>>(&mut self, with: I) {
        self.0.extend(Into::<InstructionSet>::into(with).0);
    }

    pub fn finalize(&mut self) -> Vec<Instruction> {
        self.0.clone()
    }
}

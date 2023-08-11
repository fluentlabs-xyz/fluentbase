use crate::{
    common::UntypedValue,
    engine::{
        bytecode::{
            BlockFuel,
            BranchOffset,
            BranchTableTargets,
            FuncIdx,
            GlobalIdx,
            InstrMeta,
            Instruction,
            LocalDepth,
            SignatureIdx,
        },
        CompiledFunc,
        DropKeep,
    },
};
use alloc::{slice::SliceIndex, vec::Vec};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InstructionSet {
    pub instr: Vec<Instruction>,
    pub metas: Option<Vec<InstrMeta>>,
}

macro_rules! impl_opcode {
    ($name:ident, $opcode:ident($into:ident)) => {
        pub fn $name<I: Into<$into>>(&mut self, value: I) {
            self.push(Instruction::$opcode(value.into()));
        }
    };
    ($name:ident, $opcode:ident($into:ident, $into2:ident)) => {
        pub fn $name<I: Into<$into>, J: Into<$into2>>(&mut self, value: I, value2: J) {
            self.push(Instruction::$opcode(value.into(), value2.into()));
        }
    };
    ($name:ident, $opcode:ident) => {
        pub fn $name(&mut self) {
            self.push(Instruction::$opcode);
        }
    };
}

impl From<Vec<Instruction>> for InstructionSet {
    fn from(value: Vec<Instruction>) -> Self {
        Self {
            instr: value,
            metas: None,
        }
    }
}

impl InstructionSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, opcode: Instruction) -> u32 {
        let opcode_pos = self.len();
        self.instr.push(opcode);
        opcode_pos
    }

    pub fn push_with_meta(&mut self, opcode: Instruction, meta: InstrMeta) -> u32 {
        let opcode_pos = self.push(opcode);
        let metas_len = if let Some(metas) = &mut self.metas {
            metas.push(meta);
            metas.len()
        } else {
            self.metas = Some(vec![meta]);
            1
        };
        assert_eq!(self.instr.len(), metas_len, "instr len and meta mismatched");
        opcode_pos
    }

    pub fn has_meta(&self) -> bool {
        self.metas.is_some()
    }

    pub fn get_mut<I>(&mut self, index: I) -> Option<&mut Instruction>
    where
        I: SliceIndex<[Instruction], Output = Instruction>,
    {
        self.instr.get_mut(index)
    }

    pub fn count_globals(&self) -> u32 {
        self.instr
            .iter()
            .filter_map(|opcode| match opcode {
                Instruction::GlobalGet(index) | Instruction::GlobalSet(index) => Some(index.to_u32()),
                _ => None,
            })
            .max()
            .map(|v| v + 1)
            .unwrap_or_default()
    }

    pub fn len(&self) -> u32 {
        self.instr.len() as u32
    }

    impl_opcode!(op_local_get, LocalGet(LocalDepth));
    impl_opcode!(op_local_set, LocalSet(LocalDepth));
    impl_opcode!(op_local_tee, LocalTee(LocalDepth));
    impl_opcode!(op_br, Br(BranchOffset));
    impl_opcode!(op_br_if_eqz, BrIfEqz(BranchOffset));
    impl_opcode!(op_br_if_nez, BrIfNez(BranchOffset));
    impl_opcode!(op_br_adjust, BrAdjust(BranchOffset));
    impl_opcode!(op_br_adjust_if_nez, BrAdjustIfNez(BranchOffset));
    impl_opcode!(op_br_table, BrTable(BranchTableTargets));
    impl_opcode!(op_unreachable, Unreachable);
    impl_opcode!(op_consume_fuel, ConsumeFuel(BlockFuel));
    impl_opcode!(op_return, Return(DropKeep));
    impl_opcode!(op_return_if_nez, ReturnIfNez(DropKeep));
    impl_opcode!(op_return_call_internal, ReturnCallInternal(CompiledFunc));
    impl_opcode!(op_return_call, ReturnCall(FuncIdx));
    impl_opcode!(op_return_call_indirect, ReturnCallIndirect(SignatureIdx));
    impl_opcode!(op_call_internal, CallInternal(CompiledFunc));
    impl_opcode!(op_call, Call(FuncIdx));
    impl_opcode!(op_call_indirect, CallIndirect(SignatureIdx));
    impl_opcode!(op_drop, Drop);
    impl_opcode!(op_select, Select);
    impl_opcode!(op_global_get, GlobalGet(GlobalIdx));
    impl_opcode!(op_global_set, GlobalSet(GlobalIdx));
    // TODO: "add more opcodes"
    impl_opcode!(op_i32_const, I32Const(UntypedValue));
    impl_opcode!(op_i64_const, I64Const(UntypedValue));
    impl_opcode!(op_sanitizer_stack_check, SanitizerStackCheck(i32));

    pub fn extend<I: Into<InstructionSet>>(&mut self, with: I) {
        self.instr.extend(Into::<InstructionSet>::into(with).instr);
    }

    pub fn finalize(&mut self) -> Vec<Instruction> {
        self.instr.clone()
    }
}

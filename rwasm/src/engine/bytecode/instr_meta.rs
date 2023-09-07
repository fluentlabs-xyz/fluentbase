use crate::engine::bytecode::Instruction;

type ResByteLength = usize;
type ResIsSigned = bool;

impl Instruction {
    pub fn store_instr_meta(instr: &Instruction) -> ResByteLength {
        match instr {
            Instruction::I32Store(_) => 4,
            Instruction::I32Store8(_) => 1,
            Instruction::I32Store16(_) => 2,
            Instruction::I64Store(_) => 8,
            Instruction::I64Store8(_) => 1,
            Instruction::I64Store16(_) => 2,
            Instruction::I64Store32(_) => 4,
            Instruction::F32Store(_) => 4,
            Instruction::F64Store(_) => 8,
            _ => unreachable!("unsupported opcode {:?}", instr),
        }
    }

    pub fn load_instr_meta(instr: &Instruction) -> (ResByteLength, ResIsSigned) {
        match instr {
            Instruction::I32Load(_) => (4, false),
            Instruction::I64Load(_) => (8, false),
            Instruction::F32Load(_) => (4, false),
            Instruction::F64Load(_) => (8, false),
            Instruction::I32Load8S(_) => (1, true),
            Instruction::I32Load8U(_) => (1, false),
            Instruction::I32Load16S(_) => (2, true),
            Instruction::I32Load16U(_) => (2, false),
            Instruction::I64Load8S(_) => (1, true),
            Instruction::I64Load8U(_) => (1, false),
            Instruction::I64Load16S(_) => (2, true),
            Instruction::I64Load16U(_) => (2, false),
            Instruction::I64Load32S(_) => (4, true),
            Instruction::I64Load32U(_) => (4, false),
            _ => unreachable!("unsupported opcode {:?}", instr),
        }
    }
}

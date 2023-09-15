use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::engine::bytecode::Instruction;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum ExecutionState {
    WASM_BIN,
    WASM_BREAK,
    WASM_CALL,
    WASM_CALL_HOST(SysFuncIdx),
    WASM_CONST,
    WASM_REFFUNC,
    WASM_CONVERSION,
    WASM_DROP,
    WASM_GLOBAL,
    WASM_LOAD,
    WASM_LOCAL,
    WASM_REL,
    WASM_SELECT,
    WASM_STORE,
    WASM_TEST,
    WASM_UNARY,
    WASM_TABLE_SIZE,
    WASM_TABLE_FILL,
    WASM_TABLE_GROW,
    WASM_TABLE_SET,
    WASM_TABLE_GET,
    WASM_TABLE_COPY,
    WASM_TABLE_INIT,
    WASM_BITWISE,
    WASM_EXTEND,
    WASM_MEMORY_COPY,
    WASM_MEMORY_GROW,
    WASM_MEMORY_SIZE,
    WASM_MEMORY_FILL,
    WASM_MEMORY_INIT,
    WASM_UNREACHABLE,
    WASM_SHIFT,
}

impl ExecutionState {
    pub fn to_u64(&self) -> u64 {
        match self {
            ExecutionState::WASM_BIN => 1,
            ExecutionState::WASM_BREAK => 2,
            ExecutionState::WASM_CALL => 3,
            ExecutionState::WASM_CALL_HOST(id) => 0x040000u64 + *id as u64,
            ExecutionState::WASM_CONST => 5,
            ExecutionState::WASM_CONVERSION => 6,
            ExecutionState::WASM_DROP => 7,
            ExecutionState::WASM_GLOBAL => 8,
            ExecutionState::WASM_LOAD => 9,
            ExecutionState::WASM_LOCAL => 10,
            ExecutionState::WASM_REL => 11,
            ExecutionState::WASM_SELECT => 12,
            ExecutionState::WASM_STORE => 13,
            ExecutionState::WASM_TEST => 14,
            ExecutionState::WASM_UNARY => 15,
            ExecutionState::WASM_REFFUNC => 16,
            ExecutionState::WASM_TABLE_COPY => 17,
            ExecutionState::WASM_TABLE_FILL => 18,
            ExecutionState::WASM_TABLE_GET => 19,
            ExecutionState::WASM_TABLE_GROW => 20,
            ExecutionState::WASM_TABLE_INIT => 21,
            ExecutionState::WASM_TABLE_SET => 22,
            ExecutionState::WASM_TABLE_SIZE => 23,
            ExecutionState::WASM_BITWISE => 24,
            ExecutionState::WASM_EXTEND => 25,
            ExecutionState::WASM_MEMORY_COPY => 26,
            ExecutionState::WASM_MEMORY_GROW => 27,
            ExecutionState::WASM_MEMORY_SIZE => 28,
            ExecutionState::WASM_MEMORY_FILL => 29,
            ExecutionState::WASM_MEMORY_INIT => 30,
            ExecutionState::WASM_UNREACHABLE => 31,
            ExecutionState::WASM_SHIFT => 32,
        }
    }

    pub fn from_opcode(instr: Instruction) -> ExecutionState {
        match instr {
            Instruction::Call(func_idx) => {
                return ExecutionState::WASM_CALL_HOST(SysFuncIdx::from(func_idx));
            }
            _ => {}
        }
        for state in Self::iter() {
            // TODO: "yes, I've heard about lazy static, don't understand why its not here"
            let found = state
                .responsible_opcodes()
                .iter()
                .copied()
                .find(|v| v.code_value() == instr.code_value());
            if found.is_some() {
                return state;
            }
        }
        unreachable!(
            "there is no execution state for opcode {:?}, it's not possible",
            instr
        )
    }

    pub fn responsible_opcodes(&self) -> Vec<Instruction> {
        match self {
            Self::WASM_UNREACHABLE => vec![Instruction::Unreachable],
            Self::WASM_BIN => vec![
                Instruction::I32Add,
                Instruction::I64Add,
                Instruction::I32Sub,
                Instruction::I64Sub,
                Instruction::I32Mul,
                Instruction::I64Mul,
                Instruction::I32DivS,
                Instruction::I64DivS,
                Instruction::I32DivU,
                Instruction::I64DivU,
                Instruction::I32RemS,
                Instruction::I64RemS,
                Instruction::I32RemU,
                Instruction::I64RemU,
            ],
            Self::WASM_BREAK => vec![
                Instruction::Br(Default::default()),
                Instruction::BrIfEqz(Default::default()),
                Instruction::BrIfNez(Default::default()),
                Instruction::BrAdjust(Default::default()),
                Instruction::BrAdjustIfNez(Default::default()),
            ],
            Self::WASM_CALL => vec![
                Instruction::Return(Default::default()),
                Instruction::ReturnIfNez(Default::default()),
                Instruction::ReturnCallInternal(Default::default()),
                Instruction::ReturnCallIndirectUnsafe(Default::default()),
                Instruction::CallInternal(Default::default()),
                Instruction::CallIndirectUnsafe(Default::default()),
            ],
            Self::WASM_CALL_HOST(SysFuncIdx::IMPORT_UNKNOWN) => vec![
                Instruction::ReturnCall(Default::default()),
                Instruction::Call(Default::default()),
            ],
            Self::WASM_CONST => vec![
                Instruction::I32Const(Default::default()),
                Instruction::I64Const(Default::default()),
            ],
            Self::WASM_REFFUNC => vec![Instruction::RefFunc(Default::default())],
            Self::WASM_DROP => vec![Instruction::Drop],
            Self::WASM_TEST => vec![Instruction::I32Eqz, Instruction::I64Eqz],
            Self::WASM_REL => vec![
                Instruction::I32GtU,
                Instruction::I32GeU,
                Instruction::I32LtU,
                Instruction::I32LeU,
                Instruction::I32Eq,
                Instruction::I32Ne,
                Instruction::I32GtS,
                Instruction::I32GeS,
                Instruction::I32LtS,
                Instruction::I32LeS,
                Instruction::I64GtU,
                Instruction::I64GeU,
                Instruction::I64LtU,
                Instruction::I64LeU,
                Instruction::I64Eq,
                Instruction::I64Ne,
                Instruction::I64GtS,
                Instruction::I64GeS,
                Instruction::I64LtS,
                Instruction::I64LeS,
            ],
            Self::WASM_UNARY => vec![
                Instruction::I32Ctz,
                Instruction::I64Ctz,
                Instruction::I32Clz,
                Instruction::I64Clz,
                Instruction::I32Popcnt,
                Instruction::I64Popcnt,
            ],
            Self::WASM_CONVERSION => vec![
                Instruction::I32WrapI64,
                // Instruction::I64ExtendI32U,
                // Instruction::I64ExtendI32S,
            ],
            Self::WASM_GLOBAL => vec![
                Instruction::GlobalGet(Default::default()),
                Instruction::GlobalSet(Default::default()),
            ],
            Self::WASM_LOCAL => vec![
                Instruction::LocalGet(Default::default()),
                Instruction::LocalSet(Default::default()),
                Instruction::LocalTee(Default::default()),
            ],
            Self::WASM_SELECT => vec![Instruction::Select],
            Self::WASM_TABLE_SIZE => vec![Instruction::TableSize(Default::default())],
            Self::WASM_TABLE_FILL => vec![Instruction::TableFill(Default::default())],
            Self::WASM_TABLE_GROW => vec![Instruction::TableGrow(Default::default())],
            Self::WASM_TABLE_SET => vec![Instruction::TableSet(Default::default())],
            Self::WASM_TABLE_GET => vec![Instruction::TableGet(Default::default())],
            Self::WASM_TABLE_COPY => vec![Instruction::TableCopy(Default::default())],
            Self::WASM_TABLE_INIT => vec![Instruction::TableInit(Default::default())],
            Self::WASM_STORE => vec![
                Instruction::I32Store(Default::default()),
                Instruction::I32Store8(Default::default()),
                Instruction::I32Store16(Default::default()),
                Instruction::I64Store(Default::default()),
                Instruction::I64Store8(Default::default()),
                Instruction::I64Store16(Default::default()),
                Instruction::I64Store32(Default::default()),
                // Instruction::F32Store(Default::default()),
                // Instruction::F64Store(Default::default()),
            ],
            Self::WASM_LOAD => vec![
                Instruction::I32Load(Default::default()),
                Instruction::I64Load(Default::default()),
                // Instruction::F32Load(Default::default()),
                // Instruction::F64Load(Default::default()),
                Instruction::I32Load8S(Default::default()),
                Instruction::I32Load8U(Default::default()),
                Instruction::I32Load16S(Default::default()),
                Instruction::I32Load16U(Default::default()),
                Instruction::I64Load8S(Default::default()),
                Instruction::I64Load8U(Default::default()),
                Instruction::I64Load16S(Default::default()),
                Instruction::I64Load16U(Default::default()),
                Instruction::I64Load32S(Default::default()),
                Instruction::I64Load32U(Default::default()),
            ],
            Self::WASM_BITWISE => vec![
                Instruction::I32And,
                Instruction::I64And,
                Instruction::I32Or,
                Instruction::I64Or,
                Instruction::I32Xor,
                Instruction::I64Xor,
            ],
            Self::WASM_EXTEND => vec![
                Instruction::I32Extend8S,
                Instruction::I32Extend16S,
                Instruction::I64Extend8S,
                Instruction::I64Extend16S,
                Instruction::I64Extend32S,
                Instruction::I64ExtendI32S,
                Instruction::I64ExtendI32U,
            ],
            Self::WASM_SHIFT => vec![
                Instruction::I32Shl,
                Instruction::I32ShrS,
                Instruction::I32ShrU,
                Instruction::I64Shl,
                Instruction::I64ShrS,
                Instruction::I64ShrU,
                Instruction::I32Rotl,
                Instruction::I32Rotr,
                Instruction::I64Rotl,
                Instruction::I64Rotr,
            ],
            Self::WASM_MEMORY_COPY => vec![Instruction::MemoryCopy],
            Self::WASM_MEMORY_GROW => vec![Instruction::MemoryGrow],
            Self::WASM_MEMORY_SIZE => vec![Instruction::MemorySize],
            Self::WASM_MEMORY_FILL => vec![Instruction::MemoryFill],
            Self::WASM_MEMORY_INIT => vec![Instruction::MemoryInit(Default::default())],
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::execution_state::ExecutionState;
    use fluentbase_rwasm::engine::bytecode::Instruction;
    use log::debug;
    use std::collections::HashMap;
    use strum::IntoEnumIterator;

    #[test]
    fn calc_opcode_coverage() {
        let mut used_opcodes: HashMap<Instruction, usize> = Instruction::iter()
            .filter(|instr| {
                let opcode_str = format!("{:?}", instr);
                if opcode_str.contains("F32") || opcode_str.contains("F64") {
                    false
                } else {
                    true
                }
            })
            .map(|instr| (instr, 0usize))
            .collect();
        let mut total_used = 0usize;
        for state in ExecutionState::iter() {
            for opcode in state.responsible_opcodes() {
                let used_opcode = used_opcodes.get_mut(&opcode);
                if used_opcode.is_none() {
                    panic!("opcode is filtered: {:?}", opcode)
                }
                let used_opcode = used_opcode.unwrap();
                if *used_opcode == 1 {
                    panic!(
                        "opcode ({:?}) is used more than 1 time, its not allowed",
                        opcode
                    )
                }
                let _opcode_str = format!("{:?}", opcode);
                *used_opcode += 1;
                total_used += 1;
            }
        }
        let coverage = 100 * total_used / used_opcodes.len();
        debug!(
            "opcode coverage (based on execution state) is: {}%",
            coverage
        );
        debug!("\n not implemented opcodes:");
        for (opcode, used) in used_opcodes.iter() {
            if *used == 0 {
                debug!("- {:?}", opcode)
            }
        }
    }
}

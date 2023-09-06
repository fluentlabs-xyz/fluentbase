use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::engine::bytecode::Instruction;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum ExecutionState {
    WASM_BIN, // DONE
    WASM_BREAK,
    WASM_CALL,
    WASM_CALL_HOST(SysFuncIdx),
    WASM_CONST,      // DONE
    WASM_CONVERSION, // DONE
    WASM_DROP,       // DONE
    WASM_GLOBAL,     // DONE
    WASM_LOAD,
    WASM_LOCAL,  // DONE
    WASM_REL,    // DONE
    WASM_SELECT, // DONE
    WASM_STORE,  // DONE
    WASM_TEST,   // DONE
    WASM_UNARY,  // DONE
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
                Instruction::I64ExtendI32U,
                Instruction::I64ExtendI32S,
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
            Self::WASM_STORE => vec![
                Instruction::I32Store(Default::default()),
                Instruction::I32Store8(Default::default()),
                Instruction::I32Store16(Default::default()),
                Instruction::I64Store(Default::default()),
                Instruction::I64Store8(Default::default()),
                Instruction::I64Store16(Default::default()),
                Instruction::I64Store32(Default::default()),
                Instruction::F32Store(Default::default()),
                Instruction::F64Store(Default::default()),
            ],
            Self::WASM_LOAD => vec![
                Instruction::I32Load(Default::default()),
                Instruction::I32Load8U(Default::default()),
                Instruction::I32Load8S(Default::default()),
                Instruction::I32Load16U(Default::default()),
                Instruction::I32Load16S(Default::default()),
                Instruction::I64Load(Default::default()),
                Instruction::I64Load8U(Default::default()),
                Instruction::I64Load8S(Default::default()),
                Instruction::I64Load16U(Default::default()),
                Instruction::I64Load16S(Default::default()),
                Instruction::I64Load32U(Default::default()),
                Instruction::I64Load32S(Default::default()),
                Instruction::F32Load(Default::default()),
                Instruction::F64Load(Default::default()),
            ],
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::execution_state::ExecutionState;
    use fluentbase_rwasm::engine::bytecode::Instruction;
    use std::collections::HashMap;
    use strum::IntoEnumIterator;

    #[test]
    fn calc_opcode_coverage() {
        let mut used_opcodes: HashMap<Instruction, usize> =
            Instruction::iter().map(|instr| (instr, 0usize)).collect();
        let mut total_used = 0usize;
        for state in ExecutionState::iter() {
            for opcode in state.responsible_opcodes() {
                let used_opcode = used_opcodes.get_mut(&opcode).unwrap();
                if *used_opcode == 1 {
                    panic!(
                        "opcode ({:?}) is used more than 1 time, its not allowed",
                        opcode
                    )
                }
                *used_opcode += 1;
                total_used += 1;
            }
        }
        let coverage = 100 * total_used / used_opcodes.len();
        println!(
            "opcode coverage (based on execution state) is: {}%",
            coverage
        )
    }
}

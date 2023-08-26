use fluentbase_rwasm::engine::bytecode::Instruction;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum ExecutionState {
    WASM_BIN,
    WASM_BREAK,
    WASM_CALL,
    WASM_CONST,
    WASM_CONVERSION,
    WASM_DROP,
    WASM_END,
    WASM_GLOBAL,
    WASM_LOAD,
    WASM_LOCAL,
    WASM_REL,
    WASM_SELECT,
    WASM_STORE,
    WASM_TEST,
    WASM_UNARY,
}

impl ExecutionState {
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
                Instruction::Return(Default::default()),
                Instruction::Br(Default::default()),
                Instruction::BrIfEqz(Default::default()),
                Instruction::BrTable(Default::default()),
            ],
            Self::WASM_CONST => vec![
                Instruction::I32Const(Default::default()),
                Instruction::I64Const(Default::default()),
            ],
            Self::WASM_CALL => vec![
                Instruction::Call(Default::default()),
                Instruction::CallIndirect(Default::default()),
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
            _ => unreachable!("not supported execution state {:?}", self),
        }
    }

    pub fn instruction_matches(&self) -> Vec<Instruction> {
        match self {
            ExecutionState::WASM_CONST => vec![],
            _ => unreachable!("not supported state {:?}", self),
        }
    }
}

use fluentbase_rwasm::engine::bytecode::Instruction;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum ExecutionState {
    WASM_BIN, // DONE
    WASM_BREAK,
    WASM_CALL,
    WASM_CONST,      // DONE
    WASM_CONVERSION, // DONE
    WASM_DROP,       // DONE
    WASM_GLOBAL,     // DONE
    WASM_LOAD,
    WASM_LOCAL,  // DONE
    WASM_REL,    // DONE
    WASM_SELECT, // DONE
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
}

impl ExecutionState {
    pub fn from_opcode(instr: Instruction) -> ExecutionState {
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
                Instruction::CallIndirectUnsafe(Default::default()),
                Instruction::ReturnCall(Default::default()),
                Instruction::ReturnCallIndirectUnsafe(Default::default()),
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
            Self::WASM_TABLE_SIZE => vec![Instruction::TableSize(Default::default())],
            Self::WASM_TABLE_FILL => vec![Instruction::TableFill(Default::default())],
            Self::WASM_TABLE_GROW => vec![Instruction::TableGrow(Default::default())],
            Self::WASM_TABLE_SET => vec![Instruction::TableSet(Default::default())],
            Self::WASM_TABLE_GET => vec![Instruction::TableGet(Default::default())],
            Self::WASM_TABLE_COPY => vec![Instruction::TableCopy(Default::default())],
            Self::WASM_TABLE_INIT => vec![Instruction::TableInit(Default::default())],
            _ => vec![],
        }
    }
}

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
    pub fn instruction_matches(&self) -> Vec<Instruction> {
        match self {
            ExecutionState::WASM_CONST => vec![],
            _ => unreachable!("not supported state {:?}", self),
        }
    }
}

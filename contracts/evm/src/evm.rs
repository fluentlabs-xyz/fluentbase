use crate::{
    evm::{
        gas::Gas,
        jump_map::AnalyzedBytecode,
        memory::SharedMemory,
        result::{InstructionResult, InterpreterResult},
        stack::Stack,
    },
    instruction_table::make_instruction_table,
};
use fluentbase_sdk::{Bytes, ContractContextReader, SharedAPI};

pub mod gas;
pub mod i256;
pub mod jump_map;
pub mod macros;
pub mod memory;
pub mod result;
pub mod stack;
pub mod utils;

pub struct EVM<'a, SDK: SharedAPI> {
    pub(crate) sdk: &'a mut SDK,
    pub(crate) analyzed_bytecode: AnalyzedBytecode,
    pub(crate) input: &'a [u8],
    pub(crate) gas: Gas,
    pub(crate) ip: *const u8,
    pub(crate) state: InstructionResult,
    pub(crate) return_data_buffer: Bytes,
    pub(crate) is_static: bool,
    pub(crate) output: Option<InterpreterResult>,
    pub(crate) memory: SharedMemory,
    pub(crate) stack: Stack,
}

impl<'a, SDK: SharedAPI> EVM<'a, SDK> {
    pub fn new(sdk: &'a mut SDK, bytecode: &'a [u8], input: &'a [u8], gas_limit: u64) -> Self {
        let is_static = sdk.context().contract_is_static();
        let analyzed_bytecode = AnalyzedBytecode::new(bytecode);
        let ip = analyzed_bytecode.bytecode.as_ptr();
        let gas = Gas::new(gas_limit);
        Self {
            sdk,
            analyzed_bytecode,
            input,
            gas,
            ip,
            state: InstructionResult::Continue,
            return_data_buffer: Default::default(),
            is_static,
            output: None,
            memory: Default::default(),
            stack: Default::default(),
        }
    }

    pub fn exec(&mut self) -> InterpreterResult {
        let instruction_table = make_instruction_table::<SDK>();
        while self.state == InstructionResult::Continue {
            let opcode = unsafe { *self.ip };
            self.ip = unsafe { self.ip.offset(1) };
            instruction_table[opcode as usize](self);
        }
        if let Some(output) = self.output.take() {
            return output;
        }
        InterpreterResult {
            result: self.state,
            output: Bytes::new(),
            gas: self.gas,
        }
    }

    pub fn program_counter(&self) -> usize {
        unsafe {
            self.ip
                .offset_from(self.analyzed_bytecode.bytecode.as_ptr()) as usize
        }
    }
}

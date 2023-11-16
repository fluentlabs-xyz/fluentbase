use alloy_primitives::Bytes;

use fluentbase_rwasm::rwasm::InstructionSet;

use crate::primitives::Bytecode;
use crate::translator::host::host_impl::HostImpl;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::instructions::opcode::make_instruction_table;
use crate::translator::translator::contract::Contract;
use crate::translator::translator::Translator;

#[derive(Default)]
pub struct EvmCompiler<'a> {
    pub evm_bytecode: &'a [u8],
    pub instruction_set: InstructionSet,
}

impl<'a> EvmCompiler<'a> {
    pub fn new(evm_bytecode: &'a [u8]) -> Self {
        Self {
            evm_bytecode,
            ..Default::default()
        }
    }

    pub fn translate(&mut self) -> InstructionResult {
        let evm_bytecode =
            Bytecode::new_raw(Bytes::copy_from_slice(self.evm_bytecode)).to_checked();

        let contract = Box::new(Contract::new(evm_bytecode));
        let mut translator = Translator::new(contract);

        let mut host = HostImpl::new(&mut self.instruction_set);
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host);
        res
    }
}

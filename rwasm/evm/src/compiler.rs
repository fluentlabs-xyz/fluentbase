use crate::{
    primitives::Bytecode,
    translator::{
        host::host_impl::HostImpl,
        instruction_result::InstructionResult,
        instructions::opcode::make_instruction_table,
        translator::{contract::Contract, Translator},
    },
};
use alloy_primitives::Bytes;
use fluentbase_rwasm::rwasm::{ImportLinker, InstructionSet};

#[derive()]
pub struct EvmCompiler<'a> {
    pub evm_bytecode: &'a [u8],
    pub instruction_set: InstructionSet,
    import_linker: &'a ImportLinker,
    inject_fuel_consumption: bool,
}

impl<'a> EvmCompiler<'a> {
    pub fn new(
        import_linker: &'a ImportLinker,
        inject_fuel_consumption: bool,
        evm_bytecode: &'a [u8],
    ) -> Self {
        Self {
            evm_bytecode,
            inject_fuel_consumption,
            import_linker,
            instruction_set: Default::default(),
        }
    }

    pub fn get_mut_instruction_set(&mut self) -> &InstructionSet {
        &self.instruction_set
    }

    pub fn translate(&mut self) -> InstructionResult {
        let evm_bytecode =
            Bytecode::new_raw(Bytes::copy_from_slice(self.evm_bytecode)).to_checked();

        let contract = Box::new(Contract::new(evm_bytecode));
        let mut translator =
            Translator::new(self.import_linker, self.inject_fuel_consumption, contract);

        let mut host = HostImpl::new(&mut self.instruction_set);
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host);
        res
    }

    pub fn inject_fuel_consumption(&mut self, inject_fuel_consumption: bool) {
        self.inject_fuel_consumption = inject_fuel_consumption
    }
}

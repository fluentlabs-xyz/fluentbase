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
use fluentbase_rwasm::rwasm::{instruction::INSTRUCTION_BYTES, ImportLinker, InstructionSet};

#[derive()]
pub struct EvmCompiler<'a> {
    pub evm_bytecode: &'a [u8],
    pub instruction_set: InstructionSet,
    pub instruction_set_entry_offset: Option<usize>,
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
            instruction_set_entry_offset: None,
        }
    }

    pub fn instruction_set(&mut self) -> &InstructionSet {
        &self.instruction_set
    }

    pub fn compile(&mut self) -> InstructionResult {
        let evm_bytecode =
            Bytecode::new_raw(Bytes::copy_from_slice(self.evm_bytecode)).to_checked();

        let contract = Box::new(Contract::new(evm_bytecode));
        let mut translator =
            Translator::new(self.import_linker, self.inject_fuel_consumption, contract);

        // inject subroutines
        self.instruction_set_entry_offset =
            Some(translator.subroutines_instruction_set().instr.len() + 1);
        self.instruction_set
            .op_br(self.instruction_set_entry_offset.unwrap() as i32 * INSTRUCTION_BYTES as i32);
        // translator.subroutines_instruction_set().fix_br_offsets(
        //     None,
        //     None,
        //     self.instruction_set.len() as i32,
        // );
        self.instruction_set
            .instr
            .extend(&translator.subroutines_instruction_set().instr);
        // self.instruction_set_entry_offset = Some(self.instruction_set.len().as_usize());

        // TODO move it somewhere else
        self.instruction_set.op_i32_const(100);
        self.instruction_set.op_memory_grow();
        self.instruction_set.op_drop();

        let mut host = HostImpl::new(&mut self.instruction_set);
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host);
        res
    }

    pub fn inject_fuel_consumption(&mut self, inject_fuel_consumption: bool) {
        self.inject_fuel_consumption = inject_fuel_consumption
    }
}

use crate::{
    primitives::Bytecode,
    translator::{
        host::host_impl::HostImpl,
        instruction_result::InstructionResult,
        instructions::opcode::make_instruction_table,
        translator::{contract::Contract, SubroutineData, Translator},
    },
};
use alloc::boxed::Box;
use alloy_primitives::Bytes;
use fluentbase_rwasm::rwasm::{ImportLinker, InstructionSet, FUNC_SOURCE_MAP_ENTRYPOINT_IDX};

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

    pub fn run(
        &mut self,
        preamble: Option<&InstructionSet>,
        postamble: Option<&InstructionSet>,
    ) -> InstructionResult {
        let evm_bytecode =
            Bytecode::new_raw(Bytes::copy_from_slice(self.evm_bytecode)).to_checked();

        let contract = Box::new(Contract::new(evm_bytecode));
        let mut translator =
            Translator::new(self.import_linker, self.inject_fuel_consumption, contract);

        let mut instruction_set = InstructionSet::new();

        instruction_set.op_magic_prefix([0x00; 8]);

        let init_code_data = translator.subroutine_data(FUNC_SOURCE_MAP_ENTRYPOINT_IDX);
        let SubroutineData {
            begin_offset: init_code_begin_offset,
            length: init_code_length,
            ..
        } = init_code_data.cloned().unwrap_or_default();

        let instruction_set_entry_offset: usize =
            translator.subroutines_instruction_set().len() as usize + 1 - init_code_length;
        self.instruction_set_entry_offset = Some(instruction_set_entry_offset);
        instruction_set.op_br(instruction_set_entry_offset as i32);

        let mut subroutines_instruction_set = translator.subroutines_instruction_set().clone();
        let offset_change = (instruction_set.len() + init_code_begin_offset as u32) as i32;
        subroutines_instruction_set.fix_br_indirect_offset(
            Some(0),
            Some(subroutines_instruction_set.len() as usize - 1),
            offset_change,
        );
        instruction_set.extend(&subroutines_instruction_set);

        preamble.map(|v| {
            instruction_set.extend(&v);
        });

        let mut host = HostImpl::new();
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host, instruction_set);

        let mut instruction_set = translator.take_instruction_set();

        postamble.map(|v| {
            instruction_set.extend(&v);
        });

        self.instruction_set = instruction_set;

        res
    }

    pub fn inject_fuel_consumption(&mut self, inject_fuel_consumption: bool) {
        self.inject_fuel_consumption = inject_fuel_consumption
    }
}

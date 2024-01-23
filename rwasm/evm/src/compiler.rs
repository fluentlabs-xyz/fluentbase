use crate::{
    primitives::Bytecode,
    translator::{
        host::host_impl::HostImpl,
        instruction_result::InstructionResult,
        instructions::opcode::make_instruction_table,
        translator::{contract::Contract, Translator},
    },
};
use alloc::boxed::Box;
use alloy_primitives::Bytes;
use fluentbase_rwasm::rwasm::{ImportLinker, InstructionSet};

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

        let instruction_set_entry_offset = translator.subroutines_instruction_set().instr.len() + 1;
        self.instruction_set_entry_offset = Some(instruction_set_entry_offset);
        instruction_set.op_br(instruction_set_entry_offset as i32);

        let mut subroutines_instruction_set = translator.subroutines_instruction_set().clone();
        for (_opcode, data) in translator.opcode_to_subroutine_data() {
            subroutines_instruction_set.fix_br_indirect_offset(
                Some(data.begin_offset),
                Some(data.end_offset),
                (instruction_set.len() + data.begin_offset as u32) as i32,
            );
            // 'end_offset' now points to the end of 1 solid file
            break;
        }
        instruction_set
            .instr
            .extend(&subroutines_instruction_set.instr);

        preamble.map(|v| {
            instruction_set.instr.extend(&v.instr);
        });

        let mut host = HostImpl::new();
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host, instruction_set);

        let mut instruction_set = translator.take_instruction_set();

        postamble.map(|v| {
            instruction_set.instr.extend(&v.instr);
        });

        self.instruction_set = instruction_set;

        res
    }

    pub fn inject_fuel_consumption(&mut self, inject_fuel_consumption: bool) {
        self.inject_fuel_consumption = inject_fuel_consumption
    }
}

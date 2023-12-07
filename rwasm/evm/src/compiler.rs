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
use fluentbase_rwasm::rwasm::{instruction::INSTRUCTION_SIZE_BYTES, ImportLinker, InstructionSet};

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

    pub fn compile(
        &mut self,
        preamble: Option<&InstructionSet>,
        postamble: Option<&InstructionSet>,
    ) -> InstructionResult {
        let evm_bytecode =
            Bytecode::new_raw(Bytes::copy_from_slice(self.evm_bytecode)).to_checked();

        let contract = Box::new(Contract::new(evm_bytecode));
        let mut translator =
            Translator::new(self.import_linker, self.inject_fuel_consumption, contract);

        // inject subroutines
        self.instruction_set_entry_offset =
            Some(translator.subroutines_instruction_set().instr.len() + 1);
        self.instruction_set
            .op_br(self.instruction_set_entry_offset.unwrap() as i32);
        let mut subroutines_instruction_set = translator.subroutines_instruction_set().clone();
        // for (_opcode, (offset_start, offset_end)) in translator.opcode_to_subroutine_meta() {
        //     subroutines_instruction_set.fix_br_offsets(
        //         Some(*offset_start),
        //         Some(*offset_end),
        //         ((self.instruction_set.len() + *offset_start as u32) as i32)
        //             * INSTRUCTION_SIZE_BYTES as i32,
        //     );
        // }
        self.instruction_set
            .instr
            .extend(&subroutines_instruction_set.instr);

        preamble.map(|v| {
            self.instruction_set.instr.extend(&v.instr);
        });

        let mut host = HostImpl::new(&mut self.instruction_set);
        let instruction_table = make_instruction_table::<HostImpl>();
        let res = translator.run(&instruction_table, &mut host);

        postamble.map(|v| {
            self.instruction_set.instr.extend(&v.instr);
        });

        res
    }

    pub fn inject_fuel_consumption(&mut self, inject_fuel_consumption: bool) {
        self.inject_fuel_consumption = inject_fuel_consumption
    }
}

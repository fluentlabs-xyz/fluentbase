use crate::{
    translator::{
        host::Host,
        instruction_result::InstructionResult,
        instructions::{
            control::{
                JUMPI_INSTRUCTIONS_BEFORE_BR_INDIRECT_ARG,
                JUMP_INSTRUCTIONS_BEFORE_BR_INDIRECT_ARG,
            },
            opcode,
        },
        translator::{contract::Contract, stack::Stack},
    },
    utilities::invalid_op_gen,
};
use alloc::{boxed::Box, vec, vec::Vec};
pub use analysis::BytecodeLocked;
use core::marker::PhantomData;
use fluentbase_rwasm::{
    common::UntypedValue,
    engine::bytecode::Instruction,
    rwasm::{BinaryFormat, ImportLinker, InstructionSet, ReducedModule},
};
use hashbrown::HashMap;
#[cfg(test)]
use log::debug;

pub mod analysis;
pub mod contract;
pub mod stack;

#[derive()]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_pointer_prev: *const u8,
    pub instruction_result: InstructionResult,
    import_linker: &'a ImportLinker,
    opcode_to_subroutine_data: HashMap<u8, SubroutineData>,
    inject_fuel_consumption: bool,
    subroutines_instruction_set: InstructionSet,
    _lifetime: PhantomData<&'a ()>,
    native_offset_to_rwasm_instr_offset: HashMap<usize, (usize, usize)>,
    jumps: Vec<JumpMetadata>, // item: (opcode, pc_from, pc_to)
    pub stack: Stack,
    result_instruction_set: Option<InstructionSet>,
}

struct JumpMetadata {
    pub opcode: u8,
    pub pc_from: usize,
    pub pc_to: usize,
}

pub struct SubroutineData {
    pub rel_entry_offset: u32,
    pub begin_offset: usize,
    pub end_offset: usize,
}

pub struct SubroutineMeta {
    pub begin_offset: usize,
    pub end_offset: usize,
}

impl<'a> Translator<'a> {
    pub fn new(
        import_linker: &'a ImportLinker,
        inject_fuel_consumption: bool,
        contract: Box<Contract>,
    ) -> Self {
        let mut s = Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            instruction_pointer_prev: contract.bytecode.as_ptr(),
            contract,
            instruction_result: InstructionResult::Continue,
            import_linker,
            opcode_to_subroutine_data: Default::default(),
            inject_fuel_consumption,
            subroutines_instruction_set: Default::default(),
            native_offset_to_rwasm_instr_offset: Default::default(),
            _lifetime: Default::default(),
            jumps: Default::default(),
            stack: Stack::new(),
            result_instruction_set: None,
        };
        s.init_code_snippets();
        s
    }

    #[inline]
    pub fn result_instruction_set(&self) -> &InstructionSet {
        self.result_instruction_set.as_ref().unwrap()
    }

    #[inline]
    pub fn result_instruction_set_mut(&mut self) -> &mut InstructionSet {
        self.result_instruction_set.as_mut().unwrap()
    }

    #[inline]
    pub fn take_instruction_set(&mut self) -> InstructionSet {
        let instruction_set = self.result_instruction_set.take();
        instruction_set.unwrap()
    }

    #[inline]
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    pub fn jumps_add(&mut self, opcode: u8, pc_from: usize, pc_to: usize) -> usize {
        self.jumps.push(JumpMetadata {
            opcode,
            pc_from,
            pc_to,
        });
        self.jumps.len()
    }

    pub fn jumps_reset(&mut self) {
        self.jumps.clear()
    }

    fn jumps_apply<H: Host>(&mut self, host: &mut H) {
        let mut idx_to_new_instruction: Vec<(usize, Instruction)> = vec![];
        for JumpMetadata {
            opcode,
            pc_from,
            pc_to,
        } in self.jumps.iter()
        {
            let is_offsets_from = self
                .native_offset_to_rwasm_instr_offset
                .get(pc_from)
                .unwrap()
                .clone();
            let is_offsets_to = self
                .native_offset_to_rwasm_instr_offset
                .get(pc_to)
                .cloned()
                .unwrap();
            let jump_rel_offset = is_offsets_to.0 as i32 - is_offsets_from.1 as i32 - 1;

            let aux_idx: usize = match *opcode {
                opcode::JUMP => JUMP_INSTRUCTIONS_BEFORE_BR_INDIRECT_ARG,
                opcode::JUMPI => JUMPI_INSTRUCTIONS_BEFORE_BR_INDIRECT_ARG,
                _ => {
                    if cfg!(test) {
                        panic!("unsupported opcode: {}", opcode);
                    }
                    invalid_op_gen(self);
                    return;
                }
            };
            let idx = is_offsets_from.0 + aux_idx; // TODO replace form_sp_drop_u256
            let is = self.result_instruction_set();
            if let Instruction::I64Const(v) = is.instr[idx] {
                let new_value = UntypedValue::from(v.as_i32() + jump_rel_offset);
                idx_to_new_instruction.push((idx, Instruction::I64Const(new_value)));
            } else {
                if cfg!(test) {
                    panic!("expected Instruction::I64Const");
                }
                invalid_op_gen(self);
                return;
            }
        }

        let is = self.result_instruction_set_mut();
        for (idx, new_val) in idx_to_new_instruction {
            is.instr[idx] = new_val;
        }
    }

    pub fn get_import_linker(&self) -> &'a ImportLinker {
        self.import_linker
    }

    #[inline]
    pub fn instruction_prev(&self) -> u8 {
        unsafe { *(self.instruction_pointer.sub(1)) }
    }

    #[inline]
    pub fn opcode_prev(&self) -> u8 {
        unsafe { *self.instruction_pointer_prev }
    }

    #[inline]
    pub fn instruction_cur(&self) -> u8 {
        unsafe { *self.instruction_pointer }
    }

    #[inline]
    pub fn instruction_at_pc(&self, pc: usize) -> Option<u8> {
        if pc >= self.program_counter_max() {
            return None;
        }
        unsafe { Some(*self.contract.bytecode.as_ptr().offset(pc as isize)) }
    }

    #[inline]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    #[inline]
    pub fn program_counter(&self) -> usize {
        // SAFETY: `instruction_pointer` should be at an offset from the start of the bytecode.
        // In practice this is always true unless a caller modifies the `instruction_pointer` field
        // manually.
        unsafe {
            self.instruction_pointer
                .offset_from(self.contract.bytecode.as_ptr()) as usize
        }
    }

    #[inline]
    pub fn program_counter_max(&self) -> usize {
        self.contract.bytecode.len()
    }

    #[inline]
    pub fn program_counter_prev(&self) -> usize {
        unsafe {
            self.instruction_pointer_prev
                .offset_from(self.contract.bytecode.as_ptr()) as usize
        }
    }

    #[inline(always)]
    pub fn step<FN, H: Host>(&mut self, instruction_table: &[FN; 256], host: &mut H)
    where
        FN: Fn(&mut Translator<'_>, &mut H),
    {
        let opcode = self.instruction_cur();
        let pc = self.program_counter();

        let instruction_pointer = self.instruction_pointer;
        self.instruction_pointer_inc(1);

        let is_offset_start = self.result_instruction_set_mut().len() as usize;
        instruction_table[opcode as usize](self, host);
        let is_offset_end = self.result_instruction_set_mut().len() as usize - 1;
        self.native_offset_to_rwasm_instr_offset
            .insert(pc, (is_offset_start, is_offset_end));
        self.instruction_pointer_prev = instruction_pointer;

        #[cfg(test)]
        debug!(
            "translator opcode:{} pc:{} is_offset(start:{}..end:{})",
            opcode, pc, is_offset_start, is_offset_end
        );
    }

    pub fn instruction_pointer_inc(&mut self, offset: usize) {
        // Safety: In analysis we are doing padding of bytecode so that we are sure that last
        // byte instruction is STOP so we are safe to just increment program_counter bcs on last
        // instruction it will do noop and just stop execution of this contract
        self.instruction_pointer = unsafe { self.instruction_pointer.offset(offset as isize) };
    }

    pub fn get_bytecode_slice(&self, rel_offset: Option<isize>, len: usize) -> &[u8] {
        if let Some(offset) = rel_offset {
            unsafe { core::slice::from_raw_parts(self.instruction_pointer.offset(offset), len) }
        } else {
            unsafe { core::slice::from_raw_parts(self.instruction_pointer, len) }
        }
    }

    pub fn get_bytecode_byte(&self, offset: Option<isize>) -> u8 {
        if let Some(offset) = offset {
            unsafe { *self.instruction_pointer.offset(offset) }
        } else {
            unsafe { *self.instruction_pointer }
        }
    }

    pub fn run<FN, H: Host>(
        &mut self,
        instruction_table: &[FN; 256],
        host: &mut H,
        result_instruction_set: InstructionSet,
    ) -> InstructionResult
    where
        FN: Fn(&mut Translator<'_>, &mut H),
    {
        self.result_instruction_set = Some(result_instruction_set);
        while self.instruction_result == InstructionResult::Continue {
            self.step(instruction_table, host);
        }
        self.jumps_apply(host);
        self.instruction_result
    }

    fn init_code_snippets(&mut self) {
        let opcode_to_entry = include!("../../../code-snippets/bin/solid_file.rs").as_slice();
        let mut initiate_subroutines_solid_file = |rwasm_binary: &[u8]| {
            let instruction_set = ReducedModule::new(&rwasm_binary)
                .unwrap()
                .bytecode()
                .clone();
            let l = self.subroutines_instruction_set.instr.len();
            for v in opcode_to_entry {
                let opcode = v.0;
                let entry = v.1;
                let subroutine_data = SubroutineData {
                    rel_entry_offset: entry,
                    begin_offset: l,
                    end_offset: instruction_set.len() as usize - 1 + l,
                };

                if cfg!(test) {
                    if self.opcode_to_subroutine_data.contains_key(&opcode) {
                        panic!(
                            "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                            opcode, opcode
                        );
                    }
                }
                self.opcode_to_subroutine_data
                    .insert(opcode, subroutine_data);
            }
            self.subroutines_instruction_set.extend(&instruction_set);
        };

        initiate_subroutines_solid_file(
            include_bytes!("../../../code-snippets/bin/solid_file.rwasm").as_slice(),
        );
    }

    pub fn opcode_to_subroutine_data(&self) -> &HashMap<u8, SubroutineData> {
        &self.opcode_to_subroutine_data
    }

    pub fn subroutine_data(&self, opcode: u8) -> Option<&SubroutineData> {
        self.opcode_to_subroutine_data.get(&opcode)
    }

    pub fn subroutines_instruction_set(&self) -> &InstructionSet {
        &self.subroutines_instruction_set
    }
}

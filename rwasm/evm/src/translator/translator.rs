use crate::{
    translator::{
        host::Host,
        instruction_result::InstructionResult,
        instructions::{
            control::{JUMPI_PARAMS_COUNT, JUMP_PARAMS_COUNT},
            opcode,
        },
        translator::contract::Contract,
    },
    utilities::sp_drop_u256_gen,
};
pub use analysis::BytecodeLocked;
use fluentbase_rwasm::{
    common::UntypedValue,
    engine::bytecode::Instruction,
    rwasm::{BinaryFormat, ImportLinker, InstructionSet, ReducedModule},
};
use log::debug;
use std::{collections::HashMap, marker::PhantomData};

pub mod analysis;
pub mod contract;

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
    jumps_to_process: Vec<(u8, usize, usize)>, // opcode, pc_from, pc_to
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
            jumps_to_process: Default::default(),
        };
        s.init_code_snippets();
        s
    }

    pub fn jumps_to_process_add(&mut self, opcode: u8, from_pc: usize, to_pc: usize) -> usize {
        self.jumps_to_process.push((opcode, from_pc, to_pc));
        self.jumps_to_process.len()
    }

    pub fn jumps_to_process_reset(&mut self) {
        self.jumps_to_process.clear()
    }

    fn jumps_to_process_apply(&mut self, host: &mut dyn Host) {
        for (opcode, pc_from, pc_to) in self.jumps_to_process.iter() {
            let is_offsets_from = self
                .native_offset_to_rwasm_instr_offset
                .get(pc_from)
                .unwrap();
            let is_offsets_to = self.native_offset_to_rwasm_instr_offset.get(pc_to).unwrap();
            let jump_rel_offset = is_offsets_to.0 as i32 - is_offsets_from.1 as i32 - 1;

            let is = host.instruction_set();
            // TODO replace magic consts with dynamic calculation
            let aux_idx: usize = match *opcode {
                opcode::JUMP => JUMP_PARAMS_COUNT,
                opcode::JUMPI => JUMPI_PARAMS_COUNT,
                // dynamic calculation
                _ => {
                    panic!("unsupported opcode: {}", opcode)
                }
            };
            let idx = is_offsets_from.0 + aux_idx; // TODO replace form_sp_drop_u256
            debug!("translator: applying jumps fixes at idx {}", idx);
            if let Instruction::I64Const(v) = is.instr[idx] {
                let val_new = UntypedValue::from(v.as_i32() + jump_rel_offset);
                is.instr[idx] = Instruction::I64Const(val_new);
            } else {
                panic!("expected Instruction::I64Const");
            }
        }
    }

    pub fn get_import_linker(&self) -> &'a ImportLinker {
        self.import_linker
    }

    #[inline]
    pub fn opcode_prev(&self) -> u8 {
        unsafe { *(self.instruction_pointer.sub(1)) }
    }

    #[inline]
    pub fn opcode_cur(&self) -> u8 {
        unsafe { *self.instruction_pointer }
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
        let opcode = self.opcode_cur();
        let pc = self.program_counter();

        let instruction_pointer = self.instruction_pointer;
        self.instruction_pointer_inc(1);

        let is_offset_start = host.instruction_set().len() as usize;
        instruction_table[opcode as usize](self, host);
        let is_offset_end = host.instruction_set().len() as usize - 1;
        self.native_offset_to_rwasm_instr_offset
            .insert(pc, (is_offset_start, is_offset_end));
        self.instruction_pointer_prev = instruction_pointer;
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
    ) -> InstructionResult
    where
        FN: Fn(&mut Translator<'_>, &mut H),
    {
        while self.instruction_result == InstructionResult::Continue {
            self.step(instruction_table, host);
        }
        self.jumps_to_process_apply(host);
        self.instruction_result
    }

    fn init_code_snippets(&mut self) {
        let opcode_to_beginning: &[(u8, u32)] = &[
            (opcode::BYTE, 8590),
            (opcode::SHL, 10320),
            (opcode::SHR, 11096),
            (opcode::AND, 8486),
            (opcode::OR, 9203),
            (opcode::XOR, 12020),
            (opcode::NOT, 9109),
            (opcode::GT, 8813),
            (opcode::LT, 9004),
            (opcode::SGT, 10175),
            (opcode::SLT, 11875),
            (opcode::EQ, 8695),
            (opcode::SAR, 9307),
            (opcode::ISZERO, 8918),
            (opcode::ADD, 656),
            (opcode::SUB, 8254),
            (opcode::MUL, 4925),
            (opcode::DIV, 2489),
            (opcode::SDIV, 5155),
            (opcode::MOD, 4822),
            (opcode::SMOD, 7020),
            (opcode::EXP, 4062),
            (opcode::ADDMOD, 1475),
            (opcode::MULMOD, 5028),
            (opcode::SIGNEXTEND, 6770),
            (opcode::MSTORE, 15945),
            (opcode::MSTORE8, 16012),
            (opcode::MLOAD, 15769),
            (opcode::MSIZE, 15848),
            (opcode::POP, 16056),
            (opcode::DUP1, 16046),
            (opcode::DUP2, 16051),
            (opcode::SWAP1, 16074),
            (opcode::SWAP2, 16079),
            (opcode::KECCAK256, 17442),
            (opcode::ADDRESS, 16084),
            (opcode::CALLER, 16946),
            (opcode::CALLVALUE, 17098),
            (opcode::CODESIZE, 17263),
            (opcode::GAS, 17375),
            (opcode::CALLDATALOAD, 16698),
            (opcode::CALLDATASIZE, 16845),
            (opcode::CALLDATACOPY, 16234),
            (opcode::CHAINID, 13069),
            (opcode::BASEFEE, 12664),
            (opcode::BLOCKHASH, 12863),
            (opcode::COINBASE, 13227),
            (opcode::GASLIMIT, 13398),
            (opcode::NUMBER, 13536),
            (opcode::TIMESTAMP, 13811),
            (opcode::SLOAD, 13674),
            (opcode::SSTORE, 13783),
            (opcode::TSTORE, 13973),
            (opcode::TLOAD, 13949),
            (opcode::DIFFICULTY, 15331),
            (opcode::BLOBBASEFEE, 14010),
            (opcode::GASPRICE, 15469),
            (opcode::ORIGIN, 15634),
            (opcode::BLOBHASH, 14148),
            (opcode::RETURN, 12570),
            (opcode::REVERT, 12616),
        ];
        let mut initiate_subroutines_solid_file = |rwasm_binary: &[u8]| {
            let instruction_set = ReducedModule::new(&rwasm_binary)
                .unwrap()
                .bytecode()
                .clone();
            let l = self.subroutines_instruction_set.instr.len();
            for opcode_meta in opcode_to_beginning {
                let opcode = opcode_meta.0;
                let fn_beginning_offset = opcode_meta.1;
                let subroutine_data = SubroutineData {
                    rel_entry_offset: fn_beginning_offset,
                    begin_offset: l,
                    end_offset: instruction_set.len() as usize - 1 + l,
                };

                if self.opcode_to_subroutine_data.contains_key(&opcode) {
                    panic!(
                        "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                        opcode, opcode
                    );
                }
                self.opcode_to_subroutine_data
                    .insert(opcode, subroutine_data);
            }
            self.subroutines_instruction_set.extend(&instruction_set);
        };

        initiate_subroutines_solid_file(
            // include_bytes!("../../../rwasm-code-snippets/bin/bitwise_iszero.rwasm").as_slice(),
            include_bytes!("../../../rwasm-code-snippets/bin/solid_file.rwasm").as_slice(),
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

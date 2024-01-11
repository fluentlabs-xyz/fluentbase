use crate::translator::{
    host::Host,
    instruction_result::InstructionResult,
    instructions::opcode,
    translator::contract::Contract,
};
pub use analysis::BytecodeLocked;
use fluentbase_rwasm::rwasm::{BinaryFormat, ImportLinker, InstructionSet, ReducedModule};
use std::{collections::HashMap, marker::PhantomData};

pub mod analysis;
pub mod contract;

#[derive()]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_result: InstructionResult,
    import_linker: &'a ImportLinker,
    opcode_to_subroutine_data: HashMap<u8, SubroutineData>,
    inject_fuel_consumption: bool,
    subroutines_instruction_set: InstructionSet,
    _lifetime: PhantomData<&'a ()>,
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
            contract,
            instruction_result: InstructionResult::Continue,
            import_linker,
            opcode_to_subroutine_data: Default::default(),
            inject_fuel_consumption,
            subroutines_instruction_set: Default::default(),
            _lifetime: Default::default(),
        };
        s.init_code_snippets();
        s
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

    #[inline(always)]
    pub fn step<FN, H: Host>(&mut self, instruction_table: &[FN; 256], host: &mut H)
    where
        FN: Fn(&mut Translator<'_>, &mut H),
    {
        // Get current opcode.
        let opcode = unsafe { *self.instruction_pointer };

        self.instruction_pointer_inc(1);

        // execute instruction.
        (instruction_table[opcode as usize])(self, host)
    }

    pub fn instruction_pointer_inc(&mut self, offset: usize) {
        // Safety: In analysis we are doing padding of bytecode so that we are sure that last
        // byte instruction is STOP so we are safe to just increment program_counter bcs on last
        // instruction it will do noop and just stop execution of this contract
        self.instruction_pointer = unsafe { self.instruction_pointer.offset(offset as isize) };
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
        self.instruction_result
    }

    fn init_code_snippets(&mut self) {
        let opcode_to_beginning: &[(u8, u32)] = &[
            (opcode::BYTE, 8717),
            (opcode::SHL, 10449),
            (opcode::SHR, 11225),
            (opcode::AND, 8613),
            (opcode::OR, 9332),
            (opcode::XOR, 12149),
            (opcode::NOT, 9238),
            (opcode::GT, 8942),
            (opcode::LT, 9133),
            (opcode::SGT, 10304),
            (opcode::SLT, 12004),
            (opcode::EQ, 8822),
            (opcode::SAR, 9436),
            (opcode::ISZERO, 9047),
            (opcode::ADD, 783),
            (opcode::SUB, 8381),
            (opcode::MUL, 5052),
            (opcode::DIV, 2616),
            (opcode::SDIV, 5282),
            (opcode::MOD, 4949),
            (opcode::SMOD, 7147),
            (opcode::EXP, 4189),
            (opcode::ADDMOD, 1602),
            (opcode::MULMOD, 5155),
            (opcode::SIGNEXTEND, 6897),
            (opcode::MSTORE, 16132),
            (opcode::MSTORE8, 16195),
            (opcode::MLOAD, 15956),
            (opcode::MSIZE, 16035),
            (opcode::POP, 16241),
            (opcode::DUP1, 16231),
            (opcode::DUP2, 16236),
            (opcode::SWAP1, 16259),
            (opcode::SWAP2, 16264),
            (opcode::KECCAK256, 17633),
            (opcode::ADDRESS, 16269),
            (opcode::CALLER, 17134),
            (opcode::CALLVALUE, 17287),
            (opcode::CODESIZE, 17453),
            (opcode::GAS, 17566),
            (opcode::CALLDATALOAD, 16886),
            (opcode::CALLDATASIZE, 17033),
            (opcode::CALLDATACOPY, 16420),
            (opcode::CHAINID, 13200),
            (opcode::BASEFEE, 12793),
            (opcode::BLOCKHASH, 12993),
            (opcode::COINBASE, 13359),
            (opcode::GASLIMIT, 13531),
            (opcode::NUMBER, 13670),
            (opcode::TIMESTAMP, 13946),
            (opcode::SLOAD, 13809),
            (opcode::SSTORE, 13918),
            (opcode::TSTORE, 14146),
            (opcode::TLOAD, 14085),
            (opcode::DIFFICULTY, 15515),
            (opcode::BLOBBASEFEE, 14191),
            (opcode::GASPRICE, 15654),
            (opcode::ORIGIN, 15820),
            (opcode::BLOBHASH, 14330),
            (opcode::RETURN, 12699),
            (opcode::REVERT, 12745),
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

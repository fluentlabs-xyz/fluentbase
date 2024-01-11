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
            (opcode::BYTE, 8134),
            (opcode::SHL, 9675),
            (opcode::SHR, 10258),
            (opcode::AND, 8030),
            (opcode::OR, 8749),
            (opcode::XOR, 10989),
            (opcode::NOT, 8655),
            (opcode::GT, 8359),
            (opcode::LT, 8550),
            (opcode::SGT, 9530),
            (opcode::SLT, 10844),
            (opcode::EQ, 8239),
            (opcode::SAR, 8853),
            (opcode::ISZERO, 8464),
            (opcode::ADD, 395),
            (opcode::SUB, 7798),
            (opcode::MUL, 4563),
            (opcode::DIV, 2127),
            (opcode::SDIV, 4793),
            (opcode::MOD, 4460),
            (opcode::SMOD, 6564),
            (opcode::EXP, 3700),
            (opcode::ADDMOD, 1113),
            (opcode::MULMOD, 4666),
            (opcode::SIGNEXTEND, 6314),
            (opcode::MSTORE, 14852),
            (opcode::MSTORE8, 14915),
            (opcode::MLOAD, 14676),
            (opcode::MSIZE, 14755),
            (opcode::MCOPY, 0),
            (opcode::POP, 14961),
            (opcode::DUP1, 14951),
            (opcode::DUP2, 14956),
            (opcode::SWAP1, 14979),
            (opcode::SWAP2, 14984),
            (opcode::KECCAK256, 16263),
            (opcode::ADDRESS, 14989),
            (opcode::CALLER, 15764),
            (opcode::CALLVALUE, 15917),
            (opcode::CODESIZE, 16083),
            (opcode::GAS, 16196),
            (opcode::CALLDATALOAD, 15539),
            (opcode::CALLDATASIZE, 15663),
            (opcode::CALLDATACOPY, 15140),
            (opcode::CHAINID, 12026),
            (opcode::BASEFEE, 11619),
            (opcode::BLOCKHASH, 11819),
            (opcode::COINBASE, 12185),
            (opcode::GASLIMIT, 12357),
            (opcode::NUMBER, 12496),
            (opcode::TIMESTAMP, 12772),
            (opcode::SLOAD, 12635),
            (opcode::SSTORE, 12744),
            (opcode::DIFFICULTY, 14235),
            (opcode::BLOBBASEFEE, 12911),
            (opcode::GASPRICE, 14374),
            (opcode::ORIGIN, 14540),
            (opcode::BLOBHASH, 13050),
            (opcode::RETURN, 11525),
            (opcode::REVERT, 11571),
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

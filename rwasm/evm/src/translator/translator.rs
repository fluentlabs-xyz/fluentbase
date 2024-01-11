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
            (opcode::BYTE, 8459),
            (opcode::SHL, 10191),
            (opcode::SHR, 10967),
            (opcode::AND, 8355),
            (opcode::OR, 9074),
            (opcode::XOR, 11891),
            (opcode::NOT, 8980),
            (opcode::GT, 8684),
            (opcode::LT, 8875),
            (opcode::SGT, 10046),
            (opcode::SLT, 11746),
            (opcode::EQ, 8564),
            (opcode::SAR, 9178),
            (opcode::ISZERO, 8789),
            (opcode::ADD, 525),
            (opcode::SUB, 8123),
            (opcode::MUL, 4794),
            (opcode::DIV, 2358),
            (opcode::SDIV, 5024),
            (opcode::MOD, 4691),
            (opcode::SMOD, 6889),
            (opcode::EXP, 3931),
            (opcode::ADDMOD, 1344),
            (opcode::MULMOD, 4897),
            (opcode::SIGNEXTEND, 6639),
            (opcode::MSTORE, 15768),
            (opcode::MSTORE8, 15831),
            (opcode::MLOAD, 15592),
            (opcode::MSIZE, 15671),
            (opcode::POP, 15877),
            (opcode::DUP1, 15867),
            (opcode::DUP2, 15872),
            (opcode::SWAP1, 15895),
            (opcode::SWAP2, 15900),
            (opcode::KECCAK256, 17269),
            (opcode::ADDRESS, 15905),
            (opcode::CALLER, 16770),
            (opcode::CALLVALUE, 16923),
            (opcode::CODESIZE, 17089),
            (opcode::GAS, 17202),
            (opcode::CALLDATALOAD, 16522),
            (opcode::CALLDATASIZE, 16669),
            (opcode::CALLDATACOPY, 16056),
            (opcode::CHAINID, 12942),
            (opcode::BASEFEE, 12535),
            (opcode::BLOCKHASH, 12735),
            (opcode::COINBASE, 13101),
            (opcode::GASLIMIT, 13273),
            (opcode::NUMBER, 13412),
            (opcode::TIMESTAMP, 13688),
            (opcode::SLOAD, 13551),
            (opcode::SSTORE, 13660),
            (opcode::DIFFICULTY, 15151),
            (opcode::BLOBBASEFEE, 13827),
            (opcode::GASPRICE, 15290),
            (opcode::ORIGIN, 15456),
            (opcode::BLOBHASH, 13966),
            (opcode::RETURN, 12441),
            (opcode::REVERT, 12487),
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

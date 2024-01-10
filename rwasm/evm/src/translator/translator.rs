use crate::translator::{
    host::Host,
    instruction_result::InstructionResult,
    instructions::opcode,
    translator::contract::Contract,
};
pub use analysis::BytecodeLocked;
use fluentbase_rwasm::rwasm::{BinaryFormat, ImportLinker, InstructionSet, ReducedModule};
use log::debug;
use std::{collections::HashMap, fs, marker::PhantomData};

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
    pub instruction_set: InstructionSet,
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
        let mut initiate_subroutines = |opcode: u8, rwasm_binary: &[u8]| {
            if self.opcode_to_subroutine_data.contains_key(&opcode) {
                panic!(
                    "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                    opcode, opcode
                );
            }
            let fn_beginning_offset = 0;
            let instruction_set = ReducedModule::new(&rwasm_binary)
                .unwrap()
                .bytecode()
                .clone();
            debug!(
                "\nsubroutine_instruction_set (opcode 0x{:x?} len {} fn_beginning_offset {}): \n{}\n",
                opcode,
                instruction_set.instr.len(),
                fn_beginning_offset,
                instruction_set.trace(),
            );
            let l = self.subroutines_instruction_set.instr.len();
            let subroutine_data = SubroutineData {
                rel_entry_offset: fn_beginning_offset,
                begin_offset: l,
                end_offset: l + instruction_set.len() as usize - 1,
                instruction_set,
            };
            self.subroutines_instruction_set
                .extend(&subroutine_data.instruction_set);
            self.opcode_to_subroutine_data
                .insert(opcode, subroutine_data);
        };

        [
            (
                opcode::EXP,
                "../rwasm-code-snippets/bin/arithmetic_exp.rwasm",
            ),
            (
                opcode::MOD,
                "../rwasm-code-snippets/bin/arithmetic_mod.rwasm",
            ),
            (
                opcode::SMOD,
                "../rwasm-code-snippets/bin/arithmetic_smod.rwasm",
            ),
            (
                opcode::MUL,
                "../rwasm-code-snippets/bin/arithmetic_mul.rwasm",
            ),
            (
                opcode::MULMOD,
                "../rwasm-code-snippets/bin/arithmetic_mulmod.rwasm",
            ),
            (
                opcode::ADD,
                "../rwasm-code-snippets/bin/arithmetic_add.rwasm",
            ),
            (
                opcode::ADDMOD,
                "../rwasm-code-snippets/bin/arithmetic_addmod.rwasm",
            ),
            (
                opcode::SIGNEXTEND,
                "../rwasm-code-snippets/bin/arithmetic_signextend.rwasm",
            ),
            (
                opcode::SUB,
                "../rwasm-code-snippets/bin/arithmetic_sub.rwasm",
            ),
            (
                opcode::DIV,
                "../rwasm-code-snippets/bin/arithmetic_div.rwasm",
            ),
            (
                opcode::SDIV,
                "../rwasm-code-snippets/bin/arithmetic_sdiv.rwasm",
            ),
            (opcode::SHL, "../rwasm-code-snippets/bin/bitwise_shl.rwasm"),
            (opcode::SHR, "../rwasm-code-snippets/bin/bitwise_shr.rwasm"),
            (opcode::NOT, "../rwasm-code-snippets/bin/bitwise_not.rwasm"),
            (opcode::AND, "../rwasm-code-snippets/bin/bitwise_and.rwasm"),
            (opcode::OR, "../rwasm-code-snippets/bin/bitwise_or.rwasm"),
            (opcode::XOR, "../rwasm-code-snippets/bin/bitwise_xor.rwasm"),
            (opcode::EQ, "../rwasm-code-snippets/bin/bitwise_eq.rwasm"),
            (opcode::LT, "../rwasm-code-snippets/bin/bitwise_lt.rwasm"),
            (opcode::SLT, "../rwasm-code-snippets/bin/bitwise_slt.rwasm"),
            (opcode::GT, "../rwasm-code-snippets/bin/bitwise_gt.rwasm"),
            (opcode::SGT, "../rwasm-code-snippets/bin/bitwise_sgt.rwasm"),
            (opcode::SAR, "../rwasm-code-snippets/bin/bitwise_sar.rwasm"),
            (
                opcode::BYTE,
                "../rwasm-code-snippets/bin/bitwise_byte.rwasm",
            ),
            (
                opcode::ISZERO,
                "../rwasm-code-snippets/bin/bitwise_iszero.rwasm",
            ),
            (
                opcode::MSTORE,
                "../rwasm-code-snippets/bin/memory_mstore.rwasm",
            ),
            (
                opcode::MSTORE8,
                "../rwasm-code-snippets/bin/memory_mstore8.rwasm",
            ),
            (
                opcode::MLOAD,
                "../rwasm-code-snippets/bin/memory_mload.rwasm",
            ),
            (
                opcode::KECCAK256,
                "../rwasm-code-snippets/bin/system_keccak.rwasm",
            ),
            (
                opcode::ADDRESS,
                "../rwasm-code-snippets/bin/system_address.rwasm",
            ),
            (
                opcode::CALLER,
                "../rwasm-code-snippets/bin/system_caller.rwasm",
            ),
            (
                opcode::CALLVALUE,
                "../rwasm-code-snippets/bin/system_callvalue.rwasm",
            ),
            (
                opcode::CODESIZE,
                "../rwasm-code-snippets/bin/system_codesize.rwasm",
            ),
            (opcode::GAS, "../rwasm-code-snippets/bin/system_gas.rwasm"),
            (
                opcode::CALLDATALOAD,
                "../rwasm-code-snippets/bin/system_calldataload.rwasm",
            ),
            (
                opcode::CALLDATACOPY,
                "../rwasm-code-snippets/bin/system_calldatacopy.rwasm",
            ),
            (
                opcode::CALLDATASIZE,
                "../rwasm-code-snippets/bin/system_calldatasize.rwasm",
            ),
            (
                opcode::CHAINID,
                "../rwasm-code-snippets/bin/host_chainid.rwasm",
            ),
            (
                opcode::BASEFEE,
                "../rwasm-code-snippets/bin/host_basefee.rwasm",
            ),
            (
                opcode::BLOCKHASH,
                "../rwasm-code-snippets/bin/host_blockhash.rwasm",
            ),
            (
                opcode::COINBASE,
                "../rwasm-code-snippets/bin/host_coinbase.rwasm",
            ),
            (
                opcode::GASLIMIT,
                "../rwasm-code-snippets/bin/host_gaslimit.rwasm",
            ),
            (
                opcode::NUMBER,
                "../rwasm-code-snippets/bin/host_number.rwasm",
            ),
            (
                opcode::TIMESTAMP,
                "../rwasm-code-snippets/bin/host_timestamp.rwasm",
            ),
            // (opcode::SLOAD, "../rwasm-code-snippets/bin/host_sload.rwasm"), // TODO need runtime
            // binding // binding (opcode::SSTORE, "../rwasm-code-snippets/bin/
            // host_sstore.rwasm"), // TODO need runtime binding runtime
            (opcode::POP, "../rwasm-code-snippets/bin/stack_pop.rwasm"),
            (opcode::DUP1, "../rwasm-code-snippets/bin/stack_dup1.rwasm"),
            (opcode::DUP2, "../rwasm-code-snippets/bin/stack_dup2.rwasm"),
            (
                opcode::SWAP1,
                "../rwasm-code-snippets/bin/stack_swap1.rwasm",
            ),
            (
                opcode::SWAP2,
                "../rwasm-code-snippets/bin/stack_swap2.rwasm",
            ),
            (
                opcode::DIFFICULTY,
                "../rwasm-code-snippets/bin/host_env_block_difficulty.rwasm",
            ),
            (
                opcode::BLOBBASEFEE,
                "../rwasm-code-snippets/bin/host_env_blobbasefee.rwasm",
            ),
            (
                opcode::GASPRICE,
                "../rwasm-code-snippets/bin/host_env_gasprice.rwasm",
            ),
            (
                opcode::ORIGIN,
                "../rwasm-code-snippets/bin/host_env_origin.rwasm",
            ),
            (
                opcode::RETURN,
                "../rwasm-code-snippets/bin/control_return.rwasm",
            ),
        ]
        .map(|v| {
            let opcode = v.0;
            let file_path = v.1;
            let bytecode = fs::read(file_path).unwrap();
            initiate_subroutines(opcode, &bytecode);
        });
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

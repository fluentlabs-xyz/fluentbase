use crate::translator::{
    host::Host,
    instruction_result::InstructionResult,
    instructions::opcode,
    translator::contract::Contract,
};
pub use analysis::BytecodeLocked;
use fluentbase_rwasm::rwasm::{
    BinaryFormat,
    Compiler,
    FuncOrExport,
    ImportLinker,
    InstructionSet,
    ReducedModule,
};
use hashbrown::HashMap;
use log::debug;
use std::marker::PhantomData;

pub mod analysis;
pub mod contract;

#[derive()]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_result: InstructionResult,
    import_linker: &'a ImportLinker,
    opcode_to_inline_instruction_set: HashMap<u8, InstructionSet>,
    opcode_to_subroutine_instruction_set: HashMap<u8, InstructionSet>,
    inject_fuel_consumption: bool,
    opcode_to_subroutine_meta: HashMap<u8, (usize, usize)>,
    subroutines_instruction_set: InstructionSet,
    _lifetime: PhantomData<&'a ()>,
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
            opcode_to_inline_instruction_set: Default::default(),
            opcode_to_subroutine_instruction_set: Default::default(),
            inject_fuel_consumption,
            opcode_to_subroutine_meta: Default::default(),
            subroutines_instruction_set: Default::default(),
            _lifetime: Default::default(),
        };
        s.init_code_snippets();
        s.init_subroutines();
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
        let mut initiate_inlines = |opcode: u8, wasm_binary: &[u8]| {
            if self.opcode_to_inline_instruction_set.contains_key(&opcode) {
                panic!(
                    "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                    opcode, opcode
                );
            }
            let mut compiler = Compiler::new(wasm_binary, self.inject_fuel_consumption).unwrap();
            compiler.translate_func_as_inline(true);
            compiler
                .translate(Some(FuncOrExport::Func(0)), false)
                .unwrap();
            let rwasm_binary = compiler.finalize().unwrap();
            let instruction_set = ReducedModule::new(&rwasm_binary, true)
                .unwrap()
                .bytecode()
                .clone();
            debug!(
                "\ninline_instruction_set (opcode 0x{:x?} len {}): \n{}\n",
                opcode,
                instruction_set.instr.len(),
                instruction_set.trace(),
            );
            self.opcode_to_inline_instruction_set
                .insert(opcode, instruction_set);
        };
        let mut initiate_subroutines = |opcode: u8, wasm_binary: &[u8]| {
            if self
                .opcode_to_subroutine_instruction_set
                .contains_key(&opcode)
            {
                panic!(
                    "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                    opcode, opcode
                );
            }
            let mut compiler = Compiler::new(wasm_binary, self.inject_fuel_consumption).unwrap();
            compiler.swap_stack_params(false);
            compiler
                .translate(Some(FuncOrExport::Func(0)), false)
                .unwrap();
            let rwasm_binary = compiler.finalize().unwrap();
            let instruction_set = ReducedModule::new(&rwasm_binary, true)
                .unwrap()
                .bytecode()
                .clone();
            debug!(
                "\nsubroutine_instruction_set (opcode 0x{:x?} len {}): \n{}\n",
                opcode,
                instruction_set.instr.len(),
                instruction_set.trace(),
            );
            self.opcode_to_subroutine_instruction_set
                .insert(opcode, instruction_set);
        };

        [
            (opcode::ADD, "../rwasm-code-snippets/bin/arithmetic_add.wat"),
            (opcode::SUB, "../rwasm-code-snippets/bin/arithmetic_sub.wat"),
            (opcode::MUL, "../rwasm-code-snippets/bin/arithmetic_mul.wat"),
            (opcode::SHL, "../rwasm-code-snippets/bin/bitwise_shl.wat"),
            (opcode::AND, "../rwasm-code-snippets/bin/bitwise_and.wat"),
            (opcode::OR, "../rwasm-code-snippets/bin/bitwise_or.wat"),
            (opcode::XOR, "../rwasm-code-snippets/bin/bitwise_xor.wat"),
            (opcode::SHR, "../rwasm-code-snippets/bin/bitwise_shr.wat"),
            (opcode::EQ, "../rwasm-code-snippets/bin/bitwise_eq.wat"),
            (opcode::LT, "../rwasm-code-snippets/bin/bitwise_lt.wat"),
            (opcode::SLT, "../rwasm-code-snippets/bin/bitwise_slt.wat"),
            (opcode::BYTE, "../rwasm-code-snippets/bin/bitwise_byte.wat"),
            (
                opcode::ISZERO,
                "../rwasm-code-snippets/bin/bitwise_iszero.wat",
            ),
            (opcode::GT, "../rwasm-code-snippets/bin/bitwise_gt.wat"),
            (opcode::SGT, "../rwasm-code-snippets/bin/bitwise_sgt.wat"),
            (opcode::SAR, "../rwasm-code-snippets/bin/bitwise_sar.wat"),
            (
                opcode::MSTORE,
                "../rwasm-code-snippets/bin/memory_mstore.wat",
            ),
            (
                opcode::MSTORE8,
                "../rwasm-code-snippets/bin/memory_mstore8.wat",
            ),
        ]
        .map(|v| {
            let bytecode = wat::parse_file(v.1).unwrap();
            initiate_inlines(v.0, &bytecode);
            initiate_subroutines(v.0, &bytecode);
        });
    }

    fn init_subroutines(&mut self) {
        for (opcode, instruction_set) in self.opcode_to_subroutine_instruction_set.iter() {
            let l = self.subroutines_instruction_set.instr.len();
            self.opcode_to_subroutine_meta
                .insert(*opcode, (l, l + instruction_set.len() as usize - 1));
            self.subroutines_instruction_set
                .instr
                .extend(&instruction_set.instr);
        }
    }

    pub fn opcode_to_subroutine_meta(&self) -> &HashMap<u8, (usize, usize)> {
        &self.opcode_to_subroutine_meta
    }

    pub fn subroutine_meta(&self, opcode: u8) -> Option<&(usize, usize)> {
        self.opcode_to_subroutine_meta.get(&opcode)
    }

    pub fn inline_instruction_set(&mut self, opcode: u8) -> &InstructionSet {
        if let Some(is) = self.opcode_to_inline_instruction_set.get(&opcode) {
            return is;
        }
        panic!(
            "code snippet not found for opcode 0x{:x?} (decimal: {}) pc {}",
            opcode,
            opcode,
            self.program_counter()
        );
    }

    pub fn subroutines_instruction_set(&self) -> &InstructionSet {
        &self.subroutines_instruction_set
    }
}

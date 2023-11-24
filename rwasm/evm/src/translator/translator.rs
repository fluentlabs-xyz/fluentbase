use crate::translator::{
    host::Host,
    instruction_result::InstructionResult,
    instructions::opcode,
    translator::contract::Contract,
};
pub use analysis::BytecodeLocked;
use fluentbase_rwasm::rwasm::{Compiler, FuncOrExport, InstructionSet, ReducedModule};
use hashbrown::HashMap;
use log::debug;
use std::marker::PhantomData;

pub mod analysis;
pub mod contract;

#[derive(Debug)]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_result: InstructionResult,
    opcode_to_rwasm_replacer: HashMap<u8, InstructionSet>,
    inject_fuel_consumption: bool,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Translator<'a> {
    pub fn new(contract: Box<Contract>, inject_fuel_consumption: bool) -> Self {
        let mut s = Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            contract,
            instruction_result: InstructionResult::Continue,
            opcode_to_rwasm_replacer: Default::default(),
            inject_fuel_consumption,
            _lifetime: Default::default(),
        };
        s.init_code_snippets();
        s
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
        let mut initiate = |opcode: u8, wasm_binary: &[u8]| {
            if self.opcode_to_rwasm_replacer.contains_key(&opcode) {
                panic!(
                    "code snippet for opcode 0x{:x?} already exists (decimal: {})",
                    opcode, opcode
                );
            }
            let mut compiler = Compiler::new(wasm_binary, self.inject_fuel_consumption).unwrap();
            compiler.translate_func_as_inline(true);
            let rwasm_binary = compiler
                .finalize(Some(FuncOrExport::Func(0)), false)
                .unwrap();
            let instruction_set = ReducedModule::new(&rwasm_binary)
                .unwrap()
                .bytecode()
                .clone();
            if opcode == opcode::GT {
                debug!(
                    "\ncode snippet (opcode 0x{:x?} len {}): \n{}\n",
                    opcode,
                    instruction_set.instr.len(),
                    instruction_set.trace_binary(),
                );
            };
            self.opcode_to_rwasm_replacer
                .insert(opcode, instruction_set);
        };

        [
            // (opcode::SHL, "../rwasm-code-snippets/bin/bitwise_shl.wat"),
            // (opcode::SHR, "../rwasm-code-snippets/bin/bitwise_shr.wat"),
            // (opcode::BYTE, "../rwasm-code-snippets/bin/bitwise_byte.wat"),
            // (opcode::EQ, "../rwasm-code-snippets/bin/bitwise_eq.wat"),
            // (opcode::LT, "../rwasm-code-snippets/bin/bitwise_lt.wat"),
            // (opcode::SLT, "../rwasm-code-snippets/bin/bitwise_slt.wat"),
            (opcode::GT, "../rwasm-code-snippets/bin/bitwise_gt.wat"),
            // (opcode::SGT, "../rwasm-code-snippets/bin/bitwise_sgt.wat"),
            // (opcode::SAR, "../rwasm-code-snippets/bin/bitwise_sar.wat"),
            // (opcode::SUB, "../rwasm-code-snippets/bin/arithmetic_sub.wat"),
        ]
        .map(|v| {
            let bytecode = wat::parse_file(v.1).unwrap();
            initiate(v.0, &bytecode);
        });
    }

    pub fn get_code_snippet(&mut self, opcode: u8) -> &InstructionSet {
        if let Some(is) = self.opcode_to_rwasm_replacer.get(&opcode) {
            return is;
        }
        panic!(
            "code snippet not found for opcode 0x{:x?} (decimal: {}) pc {}",
            opcode,
            opcode,
            self.program_counter()
        );
    }
}

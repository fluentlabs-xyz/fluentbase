use std::marker::PhantomData;

use hashbrown::HashMap;

pub use analysis::BytecodeLocked;
use fluentbase_rwasm::rwasm::{Compiler, FuncOrExport, InstructionSet, ReducedModule};

use crate::translator::host::Host;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::instructions::opcode;
use crate::translator::translator::contract::Contract;

pub mod analysis;
pub mod contract;

#[derive(Debug)]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_result: InstructionResult,
    opcode_to_rwasm_replacer: HashMap<u8, InstructionSet>,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Translator<'a> {
    pub fn new(contract: Box<Contract>) -> Self {
        let mut s = Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            contract,
            instruction_result: InstructionResult::Continue,
            opcode_to_rwasm_replacer: Default::default(),
            _lifetime: Default::default(),
        };
        s.init_opcode_snippets();
        s
    }

    #[inline]
    pub fn current_opcode(&self) -> u8 {
        unsafe { *self.instruction_pointer }
    }

    #[inline]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    #[inline]
    pub fn program_counter(&self) -> usize {
        // SAFETY: `instruction_pointer` should be at an offset from the start of the bytecode.
        // In practice this is always true unless a caller modifies the `instruction_pointer` field manually.
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
        // byte instruction is STOP so we are safe to just increment program_counter bcs on last instruction
        // it will do noop and just stop execution of this contract
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

    fn init_opcode_snippets(&mut self) {
        let mut initiate = |opcode: u8, wasm_binary: &[u8]| {
            if self.opcode_to_rwasm_replacer.contains_key(&opcode) {
                panic!("replacer for opcode '{}' already exists", &opcode);
            }
            let rwasm_binary = Compiler::new(wasm_binary)
                .unwrap()
                .finalize(Some(FuncOrExport::Func(0)), false)
                .unwrap();
            let is = ReducedModule::new(&rwasm_binary)
                .unwrap()
                .bytecode()
                .clone();
            self.opcode_to_rwasm_replacer.insert(opcode, is);
        };

        [
            (opcode::SHL, "../rwasm-code-snippets/bin/bitwise_shl.wat"),
            (opcode::SHR, "../rwasm-code-snippets/bin/bitwise_shr.wat"),
            (opcode::BYTE, "../rwasm-code-snippets/bin/bitwise_byte.wat"),
            (opcode::LT, "../rwasm-code-snippets/bin/bitwise_lt.wat"),
            (opcode::SLT, "../rwasm-code-snippets/bin/bitwise_slt.wat"),
            (opcode::GT, "../rwasm-code-snippets/bin/bitwise_gt.wat"),
            (opcode::SGT, "../rwasm-code-snippets/bin/bitwise_sgt.wat"),
            (opcode::EQ, "../rwasm-code-snippets/bin/bitwise_eq.wat"),
            (opcode::SAR, "../rwasm-code-snippets/bin/bitwise_sar.wat"),
            (opcode::SUB, "../rwasm-code-snippets/bin/arithmetic_sub.wat"),
        ]
        .map(|v| {
            let bytecode = wat::parse_file(v.1).unwrap();
            initiate(v.0, &bytecode);
        });
    }

    pub fn get_opcode_snippet(&mut self, opcode: u8) -> &InstructionSet {
        if let Some(is) = self.opcode_to_rwasm_replacer.get(&opcode) {
            return is;
        }
        panic!("unsupported opcode: {}", opcode);
    }
}

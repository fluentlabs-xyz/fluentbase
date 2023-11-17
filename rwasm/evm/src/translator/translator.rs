use std::marker::PhantomData;

pub use analysis::BytecodeLocked;

use crate::translator::host::Host;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::translator::contract::Contract;

pub mod analysis;
pub mod contract;

#[derive(Debug)]
pub struct Translator<'a> {
    pub contract: Box<Contract>,
    pub instruction_pointer: *const u8,
    pub instruction_result: InstructionResult,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Translator<'a> {
    pub fn new(contract: Box<Contract>) -> Self {
        Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            contract,
            instruction_result: InstructionResult::Continue,
            _lifetime: Default::default(),
        }
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
}

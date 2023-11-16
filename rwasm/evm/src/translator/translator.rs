pub mod analysis;
pub mod contract;

use crate::translator::host::Host;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::translator::contract::Contract;
pub use analysis::BytecodeLocked;
use std::marker::PhantomData;

use crate::primitives::Bytes;

/// EIP-170: Contract code size limit
///
/// By default this limit is 0x6000 (~25kb)
pub const MAX_CODE_SIZE: usize = 0x6000;

/// EIP-3860: Limit and meter initcode
pub const MAX_INITCODE_SIZE: usize = 2 * MAX_CODE_SIZE;

#[derive(Debug)]
pub struct Translator<'a> {
    /// Contract information and invoking data
    pub contract: Box<Contract>,
    /// The current instruction pointer.
    pub instruction_pointer: *const u8,
    /// The execution control flag. If this is not set to `Continue`, the interpreter will stop
    /// execution.
    pub instruction_result: InstructionResult,
    /// The return data buffer for internal calls.
    pub return_data_buffer: Bytes,
    /// The offset into `self.memory` of the return data.
    ///
    /// This value must be ignored if `self.return_len` is 0.
    pub return_offset: usize,
    /// The length of the return data.
    pub return_len: usize,
    /// Whether the interpreter is in "staticcall" mode, meaning no state changes can happen.
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Translator<'a> {
    /// Create new interpreter
    pub fn new(contract: Box<Contract>) -> Self {
        Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            contract,
            instruction_result: InstructionResult::Continue,
            return_data_buffer: Bytes::new(),
            return_len: 0,
            return_offset: 0,
            _lifetime: Default::default(),
        }
    }

    /// Returns the opcode at the current instruction pointer.
    #[inline]
    pub fn current_opcode(&self) -> u8 {
        unsafe { *self.instruction_pointer }
    }

    /// Returns a reference to the contract.
    #[inline]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    /// Returns the current program counter.
    #[inline]
    pub fn program_counter(&self) -> usize {
        // SAFETY: `instruction_pointer` should be at an offset from the start of the bytecode.
        // In practice this is always true unless a caller modifies the `instruction_pointer` field manually.
        unsafe {
            self.instruction_pointer
                .offset_from(self.contract.bytecode.as_ptr()) as usize
        }
    }

    /// Executes the instruction at the current instruction pointer.
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

    /// Executes the interpreter until it returns or stops.
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

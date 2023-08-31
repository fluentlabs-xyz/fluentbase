use fluentbase_rwasm::{
    common::UntypedValue,
    engine::{bytecode::Instruction, TracerInstrState},
};
use halo2_proofs::plonk;

#[derive(Debug)]
pub enum GadgetError {
    MissingNext,
    OutOfStack,
    Plonk(plonk::Error),
}

pub const MAX_STACK_HEIGHT: usize = 1024;

#[derive(Debug)]
pub struct TraceStep(TracerInstrState, Option<TracerInstrState>);

impl TraceStep {
    pub fn new(cur: TracerInstrState, next: Option<TracerInstrState>) -> Self {
        Self(cur, next)
    }

    pub fn instr(&self) -> &Instruction {
        &self.0.opcode
    }

    pub fn stack_pointer(&self) -> u64 {
        MAX_STACK_HEIGHT as u64 - self.0.stack.len() as u64 - 1
    }

    pub fn curr_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        Ok(self.0.stack[self.0.stack.len() - nth - 1])
    }

    pub fn curr_nth_stack_addr(&self, nth: usize) -> Result<u32, GadgetError> {
        Ok((MAX_STACK_HEIGHT - self.0.stack.len() + nth) as u32)
    }

    pub fn next_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        self.1
            .clone()
            .map(|trace| trace.stack[trace.stack.len() - nth - 1])
            .ok_or(GadgetError::MissingNext)
    }

    pub fn next_nth_stack_addr(&self, nth: usize) -> Result<u32, GadgetError> {
        self.1
            .clone()
            .map(|trace| (MAX_STACK_HEIGHT - trace.stack.len() + nth) as u32)
            .ok_or(GadgetError::MissingNext)
    }

    pub fn curr(&self) -> &TracerInstrState {
        &self.0
    }

    pub fn next(&self) -> Result<&TracerInstrState, GadgetError> {
        self.1.as_ref().ok_or(GadgetError::MissingNext)
    }
}

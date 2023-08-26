pub(crate) mod op_const;
pub(crate) mod op_drop;
pub(crate) mod op_local;

use crate::{
    runtime_circuit::{constraint_builder::OpConstraintBuilder, execution_state::ExecutionState},
    util::Field,
};
use fluentbase_rwasm::{
    common::UntypedValue,
    engine::{bytecode::Instruction, TracerInstrState},
};
use halo2_proofs::{circuit::Region, plonk};

#[derive(Debug)]
pub enum GadgetError {
    MissingNext,
    OutOfStack,
    Plonk(plonk::Error),
}

#[derive(Debug)]
pub struct TraceStep(TracerInstrState, Option<TracerInstrState>);

impl TraceStep {
    pub fn new(cur: TracerInstrState, next: Option<TracerInstrState>) -> Self {
        Self(cur, next)
    }

    pub fn instr(&self) -> &Instruction {
        &self.0.opcode
    }

    pub fn curr_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        Ok(self.0.stack[self.0.stack.len() - nth - 1])
    }

    pub fn next_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        self.1
            .clone()
            .map(|trace| trace.stack[trace.stack.len() - nth - 1])
            .ok_or(GadgetError::OutOfStack)
    }

    pub fn curr(&self) -> &TracerInstrState {
        &self.0
    }

    pub fn next(&self) -> Result<&TracerInstrState, GadgetError> {
        self.1.as_ref().ok_or(GadgetError::MissingNext)
    }
}

pub trait ExecutionGadget<F: Field> {
    const NAME: &'static str;

    const EXECUTION_STATE: ExecutionState;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self;

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError>;
}

#[macro_export]
macro_rules! bail_illegal_opcode {
    ($trace:expr) => {
        unreachable!("illegal opcode place {:?}", $trace)
    };
}

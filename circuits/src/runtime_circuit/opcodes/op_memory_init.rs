use crate::{
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpMemoryInitGadget<F: Field> {
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemoryInitGadget<F> {
    const NAME: &'static str = "WASM_MEMORY_INIT";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY_INIT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        cb.assert_unreachable("this opcode is not implemented or is not supported yet");
        Self {
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        _region: &mut Region<'_, F>,
        _offset: usize,
        _trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        Ok(())
    }
}

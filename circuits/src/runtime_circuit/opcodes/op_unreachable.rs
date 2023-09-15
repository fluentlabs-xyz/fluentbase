use crate::{
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        execution_state::ExecutionState,
        opcodes::{ExecutionGadget, OpConstraintBuilder},
    },
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpUnreachableGadget<F: Field> {
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpUnreachableGadget<F> {
    const NAME: &'static str = "WASM_UNREACHABLE";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_UNREACHABLE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        cb.assert_unreachable("unreachable");
        Self {
            pd: Default::default(),
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

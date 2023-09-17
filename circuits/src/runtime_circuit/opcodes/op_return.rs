use crate::{
    constraint_builder::{Query, ToExpr},
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
pub struct OpReturnGadget<F: Field> {
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpReturnGadget<F> {
    const NAME: &'static str = "WASM_RETURN";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_RETURN;

    fn configure(_cb: &mut OpConstraintBuilder<F>) -> Self {
        Self {
            pd: Default::default(),
        }
    }

    fn configure_state_transition(
        &self,
        cb: &mut OpConstraintBuilder<F>,
        _default_stack_diff: Query<F>,
    ) {
        cb.next_pc_delta(0.expr());
        //cb.next_sp_delta(default_stack_diff);
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

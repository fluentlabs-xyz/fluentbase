use crate::{
    constraint_builder::AdviceColumnPhase2,
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, ToExpr},
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::Error};
use std::marker::PhantomData;

pub(crate) struct DropGadget<F> {
    phase2_value: AdviceColumnPhase2,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for DropGadget<F> {
    const NAME: &'static str = "WASM_DROP";

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let phase2_value = cb.query_cell_phase2();
        cb.stack_pop(phase2_value.expr());
        Self {
            phase2_value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(&self, region: &mut Region<'_, F>, offset: usize) -> Result<(), Error> {
        Ok(())
    }
}

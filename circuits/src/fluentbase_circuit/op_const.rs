use crate::constraint_builder::{AdviceColumn, ConstraintBuilder};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::Region,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

pub(crate) trait ExecutionGadget<F: FieldExt> {
    const NAME: &'static str;

    fn configure(cs: &mut ConstraintSystem<F>, cb: &mut ConstraintBuilder<F>) -> Self;

    fn assign_exec_step(&self, region: &mut Region<'_, F>, offset: usize) -> Result<(), Error>;
}

#[derive(Clone, Debug)]
pub(crate) struct WasmConstGadget<F: FieldExt> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: FieldExt> ExecutionGadget<F> for WasmConstGadget<F> {
    const NAME: &'static str = "WASM_CONST";

    fn configure(cs: &mut ConstraintSystem<F>, cb: &mut ConstraintBuilder<F>) -> Self {
        let value = cb.advice_column(cs);

        // Push the value on the stack
        // cb.stack_push(value.expr());

        Self {
            value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(&self, region: &mut Region<'_, F>, offset: usize) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn push_gadget_simple() {}
}

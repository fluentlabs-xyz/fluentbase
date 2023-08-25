use crate::{runtime_circuit::constraint_builder::OpConstraintBuilder, util::Field};
use halo2_proofs::{circuit::Region, plonk::Error};
use std::marker::PhantomData;

pub struct SameContextGadget<F: Field> {
    _pd: PhantomData<F>,
}

impl<F: Field> SameContextGadget<F> {
    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        todo!()
    }

    fn assign_exec_step(&self, region: &mut Region<'_, F>, offset: usize) -> Result<(), Error> {
        todo!()
    }
}

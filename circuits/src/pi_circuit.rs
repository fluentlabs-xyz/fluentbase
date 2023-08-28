use crate::util::Field;
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct PublicInputCircuitConfig<F: Field> {
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            marker: Default::default(),
        }
    }

    pub fn assign_bytecode(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        Ok(())
    }
}

use crate::{
    constraint_builder::{ConstraintBuilder, InstanceColumn, SelectorColumn},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct PublicInputCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    input: InstanceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let mut cb = ConstraintBuilder::new(q_enable);
        let input = cb.instance_column(cs);

        cb.build(cs);
        Self {
            q_enable,
            input,
            marker: Default::default(),
        }
    }

    pub fn assign(&self, _layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        Ok(())
    }
}

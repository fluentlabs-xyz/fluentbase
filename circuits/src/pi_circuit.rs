use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, InstanceColumn, SelectorColumn},
    poseidon_circuit::PoseidonLookup,
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
    input: AdviceColumn,
    instance: InstanceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, poseidon_lookup: &impl PoseidonLookup) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let mut cb = ConstraintBuilder::new(q_enable);
        let input = cb.advice_column(cs);
        let instance = cb.instance_column(cs);
        cs.enable_equality(input);
        cs.enable_equality(instance);
        cb.assert_equal("input == instance", input.current(), instance.current());
        cb.assert_zero("input is zero", input.current());
        cb.build(cs);
        Self {
            q_enable,
            input,
            instance,
            marker: Default::default(),
        }
    }

    pub fn expose_public(
        &self,
        layouter: &mut impl Layouter<F>,
        hash_value: F,
    ) -> Result<(), Error> {
        let assigned_cell = layouter.assign_region(
            || "",
            |mut region| {
                self.q_enable.enable(&mut region, 0);
                let assigned_cell = self.input.assign(&mut region, 0, hash_value);
                Ok(assigned_cell)
            },
        )?;
        layouter.constrain_instance(assigned_cell.cell(), self.instance.0, 0)?;
        Ok(())
    }
}

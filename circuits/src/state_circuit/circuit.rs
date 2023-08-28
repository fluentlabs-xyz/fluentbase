use crate::{
    constraint_builder::{BinaryQuery, ConstraintBuilder, SelectorColumn},
    state_circuit::rw_table::RwTable,
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct StateCircuitConfig<F: Field> {
    selector: SelectorColumn,
    rw_table: RwTable<F>,
    marker: PhantomData<F>,
}

impl<F: Field> StateCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let selector = SelectorColumn(cs.fixed_column());
        let rw_table = RwTable::configure(cs);

        let mut cb = ConstraintBuilder::new(selector);

        let is_start = || -> BinaryQuery<F> { BinaryQuery::one() };
        let is_memory = || -> BinaryQuery<F> { BinaryQuery::one() };
        let is_stack = || -> BinaryQuery<F> { BinaryQuery::one() };
        let is_global = || -> BinaryQuery<F> { BinaryQuery::one() };
        let is_table = || -> BinaryQuery<F> { BinaryQuery::one() };

        cb.condition(is_start(), |cb| {
            rw_table.build_start_constraints(cb);
        });
        cb.condition(is_memory(), |cb| {
            rw_table.build_memory_constraints(cb);
        });
        cb.condition(is_stack(), |cb| {
            rw_table.build_stack_constraints(cb);
        });
        cb.condition(is_global(), |cb| {
            rw_table.build_global_constraints(cb);
        });
        cb.condition(is_table(), |cb| {
            rw_table.build_table_constraints(cb);
        });

        cb.build(cs);

        Self {
            selector,
            rw_table,
            marker: Default::default(),
        }
    }

    pub fn assign_bytecode(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        Ok(())
    }
}

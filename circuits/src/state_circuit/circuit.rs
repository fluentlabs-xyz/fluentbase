use crate::{
    constraint_builder::{BinaryQuery, ConstraintBuilder, SelectorColumn},
    gadgets::binary_number::{BinaryNumberChip, BinaryNumberConfig},
    state_circuit::{
        rw_table::RwTable,
        tag::{RwTableTag, N_RW_TABLE_TAG_BYTES},
    },
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
    poly::Rotation,
};
use std::marker::PhantomData;

pub trait StateLookup<F: Field> {}

#[derive(Clone)]
pub struct StateCircuitConfig<F: Field> {
    selector: SelectorColumn,
    tag: BinaryNumberConfig<RwTableTag, { N_RW_TABLE_TAG_BYTES }>,
    rw_table: RwTable<F>,
    marker: PhantomData<F>,
}

impl<F: Field> StateCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let selector = SelectorColumn(cs.fixed_column());
        let rw_table = RwTable::configure(cs);

        let tag = BinaryNumberChip::configure(cs, selector, Some(rw_table.tag.current()));
        let mut cb = ConstraintBuilder::new(selector);

        let is_tag = |matches_tag: RwTableTag| -> BinaryQuery<F> {
            tag.value_equals(matches_tag, Rotation::cur())()
        };

        rw_table.build_general_constraints(&mut cb);

        cb.condition(is_tag(RwTableTag::Start), |cb| {
            rw_table.build_start_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Memory), |cb| {
            rw_table.build_memory_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Stack), |cb| {
            rw_table.build_stack_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Global), |cb| {
            rw_table.build_global_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Table), |cb| {
            rw_table.build_table_constraints(cb);
        });

        cb.build(cs);

        Self {
            selector,
            tag,
            rw_table,
            marker: Default::default(),
        }
    }

    pub fn assign_bytecode(&self, _layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        Ok(())
    }
}

use crate::{
    constraint_builder::{BinaryQuery, ConstraintBuilder, SelectorColumn},
    gadgets::{
        binary_number::{BinaryNumberChip, BinaryNumberConfig},
        range_check::RangeCheckLookup,
    },
    state_circuit::{
        lexicographic_ordering::LexicographicOrderingConfig,
        mpi_config::MpiConfig,
        rw_table::RwTable,
        sort_keys::SortKeysConfig,
        tag::{RwTableTag, N_RW_TABLE_TAG_BYTES},
    },
    util::Field,
};
use fluentbase_rwasm::engine::Tracer;
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
    sort_keys: SortKeysConfig<F>,
    lexicographic_ordering_config: LexicographicOrderingConfig,
    marker: PhantomData<F>,
}

impl<F: Field> StateCircuitConfig<F> {
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        range_check_lookup: &impl RangeCheckLookup<F>,
    ) -> Self {
        let selector = SelectorColumn(cs.fixed_column());
        let rw_table = RwTable::configure(cs);

        let tag = BinaryNumberChip::configure(cs, selector, Some(rw_table.tag.current()));
        let mut cb = ConstraintBuilder::new(selector);

        let is_tag = |matches_tag: RwTableTag| -> BinaryQuery<F> {
            tag.value_equals(matches_tag, Rotation::cur())
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

        let sort_keys = SortKeysConfig {
            id: MpiConfig::configure(cs, &mut cb, rw_table.id, range_check_lookup),
            tag,
            address: MpiConfig::configure(cs, &mut cb, rw_table.address, range_check_lookup),
            rw_counter: MpiConfig::configure(cs, &mut cb, rw_table.rw_counter, range_check_lookup),
        };

        let lexicographic_ordering_config =
            LexicographicOrderingConfig::configure(cs, &sort_keys, range_check_lookup);

        cb.build(cs);

        Self {
            selector,
            tag,
            rw_table,
            sort_keys,
            lexicographic_ordering_config,
            marker: Default::default(),
        }
    }

    pub fn assign(&self, layouter: &mut impl Layouter<F>, tracer: &Tracer) -> Result<(), Error> {
        // tracer.logs;
        Ok(())
    }
}

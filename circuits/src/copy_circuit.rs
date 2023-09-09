use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query, SelectorColumn, ToExpr},
    gadgets::binary_number::{BinaryNumberChip, BinaryNumberConfig},
    lookup_table::{PublicInputLookup, RwLookup},
    rw_builder::{
        copy_row::{CopyTableTag, N_COPY_TABLE_TAG_BITS},
        rw_row::RwTableTag,
    },
    util::Field,
};
use halo2_proofs::{circuit::Layouter, plonk::ConstraintSystem, poly::Rotation};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct CopyCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    // type of copy table entity
    tag: AdviceColumn,
    tag_bits: BinaryNumberConfig<CopyTableTag, { N_COPY_TABLE_TAG_BITS }>,
    // how many bytes to copy
    length: AdviceColumn,
    // memory ref rw counter
    rw_counter: AdviceColumn,
    // src & dst addresses
    from_address: AdviceColumn,
    to_address: AdviceColumn,
    // copy value
    value: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> CopyCircuitConfig<F> {
    fn configure(
        cs: &mut ConstraintSystem<F>,
        rw_lookup: &impl RwLookup<F>,
        pi_lookup: &impl PublicInputLookup<F>,
    ) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let tag = AdviceColumn(cs.advice_column());
        let tag_bits = BinaryNumberChip::configure(cs, q_enable, Some(tag.current()));
        let length = AdviceColumn(cs.advice_column());
        let rw_counter = AdviceColumn(cs.advice_column());
        let src_address = AdviceColumn(cs.advice_column());
        let dst_address = AdviceColumn(cs.advice_column());
        let value = AdviceColumn(cs.advice_column());

        let mut cb = ConstraintBuilder::new(q_enable);

        cb.condition(
            tag_bits.value_equals(CopyTableTag::Input, Rotation::cur()),
            |cb| {
                // lookup pi (we copy from public input)
                cb.add_lookup(
                    "public input table lookup",
                    [
                        Query::one(), // selector
                        src_address.current(),
                        value.current(),
                    ],
                    pi_lookup.lookup_input_byte(),
                );
                // lookup rw (we copy into memory)
                cb.add_lookup(
                    "rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current(),
                        1.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(),         // id
                        dst_address.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
            },
        );
        cb.condition(
            tag_bits.value_equals(CopyTableTag::Output, Rotation::cur()),
            |_cb| {
                // lookup rw

                // lookup pi
            },
        );

        cb.build(cs);

        Self {
            q_enable,
            tag,
            tag_bits,
            length,
            rw_counter,
            from_address: src_address,
            to_address: dst_address,
            value,
            pd: Default::default(),
        }
    }

    fn assign(&self, _layouter: &mut impl Layouter<F>) {}
}

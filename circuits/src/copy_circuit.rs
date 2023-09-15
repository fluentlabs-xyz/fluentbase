use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query, SelectorColumn, ToExpr},
    exec_step::ExecSteps,
    gadgets::binary_number::{BinaryNumberChip, BinaryNumberConfig},
    lookup_table::{CopyLookup, PublicInputLookup, RwLookup, N_COPY_LOOKUP_TABLE},
    only_once,
    rw_builder::{
        copy_row::{CopyRow, CopyTableTag, N_COPY_TABLE_TAG_BITS},
        rw_row::RwTableTag,
    },
    util::Field,
};
use cli_table::format::Justify;
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
    poly::Rotation,
};
use itertools::Itertools;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct CopyCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    q_first: SelectorColumn,
    q_last: SelectorColumn,
    // type of copy table entity
    tag: AdviceColumn,
    tag_bits: BinaryNumberConfig<CopyTableTag, { N_COPY_TABLE_TAG_BITS }>,
    // src & dst addresses
    from_address: AdviceColumn,
    to_address: AdviceColumn,
    // how many bytes to copy
    length: AdviceColumn,
    index: AdviceColumn,
    // memory ref rw counter
    rw_counter: AdviceColumn,
    // copy value
    value: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> CopyCircuitConfig<F> {
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        rw_lookup: &impl RwLookup<F>,
        pi_lookup: &impl PublicInputLookup<F>,
    ) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let q_first = SelectorColumn(cs.fixed_column());
        let q_last = SelectorColumn(cs.fixed_column());
        let tag = AdviceColumn(cs.advice_column());
        let tag_bits = BinaryNumberChip::configure(cs, q_enable, Some(tag.current()));
        let from_address = AdviceColumn(cs.advice_column());
        let to_address = AdviceColumn(cs.advice_column());
        let length = AdviceColumn(cs.advice_column());
        let index = AdviceColumn(cs.advice_column());
        let rw_counter = AdviceColumn(cs.advice_column());
        let value = AdviceColumn(cs.advice_column());

        let mut cb = ConstraintBuilder::new(q_enable);

        cb.condition(q_first.current(), |cb| {
            cb.assert_equal(
                "if (q_first) length==index",
                length.current(),
                index.current(),
            );
        });
        cb.condition(!q_last.current(), |cb| {
            cb.assert_equal(
                "if (!q_last) index=index-1",
                index.current(),
                index.next() + 1.expr(),
            );
            cb.assert_equal(
                "if (!q_last) rw_counter=rw_counter",
                rw_counter.current(),
                rw_counter.next(),
            );
            cb.assert_equal(
                "if (!q_last) length=length",
                length.current(),
                length.next(),
            );
        });
        cb.condition(q_first.current().and(q_last.current()), |cb| {
            cb.assert_equal(
                "if (q_first && q_last) length==1",
                length.current(),
                1.expr(),
            );
            cb.assert_equal("if (q_first && q_last) index==1", index.current(), 1.expr());
        });
        cb.condition(q_last.current(), |cb| {
            cb.assert_equal("if (q_last) index==1", index.current(), 1.expr());
        });

        cb.condition(
            tag_bits.value_equals(CopyTableTag::ReadInput, Rotation::cur()),
            |cb| {
                // lookup pi (we copy from public input)
                cb.add_lookup(
                    "public input table lookup",
                    [
                        Query::one(), // selector
                        from_address.current() + length.current() - index.current(),
                        value.current(),
                    ],
                    pi_lookup.lookup_input_byte(),
                );
                // lookup rw (we copy into memory)
                cb.add_lookup(
                    "rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current() + length.current() - index.current(),
                        1.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(),                                             // id
                        to_address.current() + length.current() - index.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
            },
        );
        cb.condition(
            tag_bits.value_equals(CopyTableTag::WriteOutput, Rotation::cur()),
            |cb| {
                // lookup rw (we copy from memory)
                cb.add_lookup(
                    "rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current() + length.current() - index.current(),
                        0.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(), // id
                        from_address.current() + length.current() - index.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
                // lookup pi (we copy into output)
                cb.add_lookup(
                    "public output table lookup",
                    [
                        Query::one(), // selector
                        to_address.current() + length.current() - index.current(),
                        value.current(),
                    ],
                    pi_lookup.lookup_output_byte(),
                );
            },
        );
        cb.condition(
            tag_bits.value_equals(CopyTableTag::CopyMemory, Rotation::cur()),
            |cb| {
                // lookup rw (we copy from memory)
                cb.add_lookup(
                    "rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current() + length.current() - index.current(),
                        0.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(), // id
                        from_address.current() + length.current() - index.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
                // lookup rw (we copy into memory)
                cb.add_lookup(
                    "rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current() + 2.expr() * length.current() - index.current(),
                        1.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(),                                             // id
                        to_address.current() + length.current() - index.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
            },
        );
        cb.condition(
            tag_bits.value_equals(CopyTableTag::FillMemory, Rotation::cur()),
            |cb| {
                // for memory fill value and from address must be the same
                cb.assert_equal(
                    "value == from_address",
                    value.current(),
                    from_address.current(),
                );
                // lookup rw (we fill memory with value)
                cb.add_lookup(
                    "memory fill, rw table lookup",
                    [
                        Query::one(), // selector
                        rw_counter.current() + length.current() - index.current(),
                        1.expr(), // is_write
                        RwTableTag::Memory.expr(),
                        Query::zero(),                                             // id
                        to_address.current() + length.current() - index.current(), // address
                        value.current(),
                    ],
                    rw_lookup.lookup_rw_table(),
                );
            },
        );

        cb.build(cs);

        Self {
            q_enable,
            q_first,
            q_last,
            tag,
            tag_bits,
            from_address,
            to_address,
            length,
            index,
            rw_counter,
            value,
            pd: Default::default(),
        }
    }

    pub fn print_copy_row_table(&self, rw_rows: &Vec<CopyRow>) {
        only_once!();
        use cli_table::{print_stdout, Cell, Style, Table};
        let table = rw_rows
            .iter()
            .map(|row| {
                vec![
                    row.tag.cell().justify(Justify::Center),
                    row.from_address.cell().justify(Justify::Center),
                    row.to_address.cell().justify(Justify::Center),
                    row.length.cell().justify(Justify::Center),
                    row.rw_counter.cell().justify(Justify::Center),
                ]
            })
            .collect_vec()
            .table()
            .title(vec![
                "tag".cell().bold(true),
                "from_address".cell().bold(true),
                "to_address".cell().bold(true),
                "length".cell().bold(true),
                "rw_counter".cell().bold(true),
            ])
            .bold(true);
        print_stdout(table).unwrap();
    }

    pub fn assign_with_region(
        &self,
        region: &mut Region<'_, F>,
        exec_steps: &ExecSteps,
    ) -> Result<(), Error> {
        let copy_rows = exec_steps.get_copy_rows();
        self.print_copy_row_table(&copy_rows);
        let mut offset = 0;
        for copy_row in copy_rows.iter() {
            self.q_first.enable(region, offset);
            let mut last_offset = offset;
            for (i, value) in copy_row.data.iter().enumerate() {
                self.q_enable.enable(region, offset);
                self.tag.assign(region, offset, copy_row.tag as u64);
                let tag_bits = BinaryNumberChip::construct(self.tag_bits);
                tag_bits.assign(region, offset, &copy_row.tag)?;
                self.from_address
                    .assign(region, offset, copy_row.from_address as u64);
                self.to_address
                    .assign(region, offset, copy_row.to_address as u64);
                self.length.assign(region, offset, copy_row.length as u64);
                self.index
                    .assign(region, offset, (copy_row.length - i as u32) as u64);
                self.rw_counter
                    .assign(region, offset, copy_row.rw_counter as u64);
                self.value.assign(region, offset, *value as u64);
                last_offset = offset;
                offset += 1;
            }
            self.q_last.enable(region, last_offset);
        }
        Ok(())
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        exec_steps: &ExecSteps,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "copy circuit",
            |mut region| self.assign_with_region(&mut region, exec_steps),
        )?;
        Ok(())
    }
}

impl<F: Field> CopyLookup<F> for CopyCircuitConfig<F> {
    fn lookup_copy_table(&self) -> [Query<F>; N_COPY_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.q_first.current().0,
            self.tag.current(),
            self.from_address.current(),
            self.to_address.current(),
            self.length.current(),
            self.rw_counter.current(),
        ]
    }
}

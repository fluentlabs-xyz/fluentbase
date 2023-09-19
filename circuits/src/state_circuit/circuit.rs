use crate::{
    constraint_builder::{BinaryQuery, ConstraintBuilder, Query, SelectorColumn},
    exec_step::ExecSteps,
    gadgets::binary_number::{BinaryNumberChip, BinaryNumberConfig},
    lookup_table::{RangeCheckLookup, RwLookup, N_RW_LOOKUP_TABLE, N_RW_PREV_LOOKUP_TABLE},
    only_once,
    rw_builder::rw_row::{RwRow, RwTableContextTag, RwTableTag, N_RW_TABLE_TAG_BITS},
    state_circuit::{
        lexicographic_ordering::{LexicographicOrderingConfig, LimbIndex},
        mpi_config::MpiConfig,
        rw_table::RwTable,
        sort_keys::SortKeysConfig,
    },
    util::Field,
};
use cli_table::format::Justify;
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
    poly::Rotation,
};
use itertools::Itertools;
use std::marker::PhantomData;
use strum::IntoEnumIterator;

#[derive(Clone)]
pub struct StateCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    tag: BinaryNumberConfig<RwTableTag, { N_RW_TABLE_TAG_BITS }>,
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
        let q_enable = SelectorColumn(cs.fixed_column());
        let rw_table = RwTable::configure(cs);

        let tag = BinaryNumberChip::configure(cs, q_enable, Some(rw_table.tag.current()));
        let mut cb = ConstraintBuilder::new(q_enable);

        let is_tag = |matches_tag: RwTableTag| -> BinaryQuery<F> {
            tag.value_equals(matches_tag, Rotation::cur())
        };

        let sort_keys = SortKeysConfig {
            id: MpiConfig::configure(cs, &mut cb, rw_table.id, range_check_lookup),
            tag,
            address: MpiConfig::configure(cs, &mut cb, rw_table.address, range_check_lookup),
            rw_counter: MpiConfig::configure(cs, &mut cb, rw_table.rw_counter, range_check_lookup),
        };

        let lexicographic_ordering_config =
            LexicographicOrderingConfig::configure(cs, &sort_keys, range_check_lookup);

        rw_table.build_general_constraints(&mut cb, &lexicographic_ordering_config);

        cb.condition(is_tag(RwTableTag::Start), |cb| {
            rw_table.build_start_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Context), |cb| {
            rw_table.build_context_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Memory), |cb| {
            rw_table.build_memory_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Stack), |cb| {
            rw_table.build_stack_constraints(cb, &lexicographic_ordering_config);
        });
        cb.condition(is_tag(RwTableTag::Global), |cb| {
            rw_table.build_global_constraints(cb);
        });
        cb.condition(is_tag(RwTableTag::Table), |cb| {
            rw_table.build_table_constraints(cb);
        });

        cb.build(cs);

        Self {
            q_enable,
            tag,
            rw_table,
            sort_keys,
            lexicographic_ordering_config,
            marker: Default::default(),
        }
    }

    pub fn assign_with_region(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        rw_row: &RwRow,
        prev_rw_row: Option<&RwRow>,
    ) -> Result<(), Error> {
        self.q_enable.enable(region, offset);
        let tag_chip = BinaryNumberChip::construct(self.sort_keys.tag);
        tag_chip.assign(region, offset, &rw_row.tag())?;
        self.sort_keys
            .rw_counter
            .assign(region, offset, rw_row.rw_counter() as u32)?;
        if let Some(id) = rw_row.id() {
            self.sort_keys.id.assign(region, offset, id as u32)?;
        }
        if let Some(address) = rw_row.address() {
            self.sort_keys.address.assign(region, offset, address)?;
        }
        if let Some(prev_rw_row) = prev_rw_row {
            let index =
                self.lexicographic_ordering_config
                    .assign(region, offset, rw_row, prev_rw_row)?;
            let is_first_access = !matches!(index, LimbIndex::RwCounter0 | LimbIndex::RwCounter1);
            self.rw_table.not_first_access.assign(
                region,
                offset,
                if is_first_access { F::zero() } else { F::one() },
            );
        }
        self.rw_table
            .assign(region, offset, rw_row, prev_rw_row.map(|v| v.value()));
        Ok(())
    }

    pub fn print_rw_rows_table(&self, rw_rows: &Vec<RwRow>, rw_meta: Vec<(Instruction, u32)>) {
        only_once!();
        use cli_table::{print_stdout, Cell, Style, Table};
        let table = rw_rows
            .iter()
            .copied()
            .enumerate()
            .map(|(i, row)| {
                let prev_value = rw_rows
                    .get((i as isize - 1) as usize)
                    .filter(|v| {
                        v.tag() == row.tag() && v.id() == row.id() && v.address() == row.address()
                    })
                    .map(|v| v.value())
                    .unwrap_or_default();
                let address = if row.tag() == RwTableTag::Context {
                    let tag = RwTableContextTag::iter()
                        .filter(|v| Into::<u32>::into(*v) == row.address().unwrap_or_default())
                        .last()
                        .unwrap();
                    format!("{} ({})", tag, row.address().unwrap_or_default())
                } else {
                    row.address().unwrap_or_default().to_string()
                };
                vec![
                    rw_meta[row.rw_counter()].1.cell().justify(Justify::Center),
                    rw_meta[row.rw_counter()].0.cell().justify(Justify::Center),
                    row.rw_counter().cell().justify(Justify::Center),
                    row.is_write().cell().justify(Justify::Center),
                    row.tag().cell().justify(Justify::Center),
                    row.id().unwrap_or_default().cell().justify(Justify::Center),
                    address.cell().justify(Justify::Center),
                    row.value().to_bits().cell().justify(Justify::Center),
                    prev_value.to_bits().cell().justify(Justify::Center),
                ]
            })
            .collect_vec()
            .table()
            .title(vec![
                "pc".cell().bold(true),
                "opcode".cell().bold(true),
                "rw_counter".cell().bold(true),
                "is_write".cell().bold(true),
                "tag".cell().bold(true),
                "id".cell().bold(true),
                "address".cell().bold(true),
                "value".cell().bold(true),
                "value_prev".cell().bold(true),
            ])
            .bold(true);
        print_stdout(table).unwrap();
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        exec_steps: &ExecSteps,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "state runtime opcodes",
            |mut region| {
                let (mut rw_rows, mut rw_meta) = exec_steps.get_rw_rows();
                rw_rows.insert(0, RwRow::Start { rw_counter: 0 });
                rw_meta.insert(0, (Instruction::Unreachable, 0));
                self.print_rw_rows_table(&rw_rows, rw_meta);
                rw_rows.sort_by_key(|row| {
                    (
                        row.tag() as u64,
                        row.id().unwrap_or_default(),
                        row.address().unwrap_or_default(),
                        row.rw_counter(),
                    )
                });
                // self.print_rw_rows_table(&rw_rows, rw_meta);
                for (offset, rw_row) in rw_rows.iter().enumerate() {
                    if offset > 0 {
                        let prev_value = rw_rows.get(offset - 1).filter(|v| {
                            v.tag() == rw_row.tag()
                                && v.id() == rw_row.id()
                                && v.address() == rw_row.address()
                        });
                        self.assign_with_region(&mut region, offset, rw_row, prev_value)?;
                    } else {
                        self.assign_with_region(&mut region, offset, rw_row, None)?;
                    }
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: Field> RwLookup<F> for StateCircuitConfig<F> {
    fn lookup_rw_table(&self) -> [Query<F>; N_RW_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.rw_table.rw_counter.current(),
            self.rw_table.is_write.current(),
            self.rw_table.tag.current(),
            self.rw_table.id.current(),
            self.rw_table.address.current(),
            self.rw_table.value.current(),
        ]
    }

    fn lookup_rw_prev_table(&self) -> [Query<F>; N_RW_PREV_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.rw_table.rw_counter.current(),
            self.rw_table.is_write.current(),
            self.rw_table.tag.current(),
            self.rw_table.id.current(),
            self.rw_table.address.current(),
            self.rw_table.value.current(),
            self.rw_table.value_prev.current(),
        ]
    }
}

use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, FixedColumn, Query},
    unrolled_bytecode::UnrolledBytecode,
    util::{poseidon_hash, Field},
};
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{Advice, Column, ConstraintSystem, Error, Fixed},
};
use poseidon_circuit::{
    hash::{PoseidonHashChip, PoseidonHashConfig, PoseidonHashTable},
    HASHABLE_DOMAIN_SPEC,
};

pub const HASH_BYTES_IN_FIELD: usize = 9;
pub const HASH_BLOCK_STEP_SIZE: usize = 2 * HASH_BYTES_IN_FIELD;

pub const N_POSEIDON_LOOKUP_TABLE: usize = 5;

pub trait PoseidonLookup {
    fn lookup_poseidon_table(&self) -> (FixedColumn, [AdviceColumn; N_POSEIDON_LOOKUP_TABLE]);
}

impl<F: Field> ConstraintBuilder<F> {
    pub fn poseidon_lookup(
        &mut self,
        name: &'static str,
        code_hash: Query<F>,
        left_input: Query<F>,
        right_input: Query<F>,
        offset: Query<F>,
        poseidon: &impl PoseidonLookup,
    ) {
        let extended_queries = [
            Query::one(),
            code_hash,
            left_input.clone(),
            right_input.clone(),
            offset * Query::Constant(F::from_u128(HASHABLE_DOMAIN_SPEC)),
        ];
        let (q_enable, [hash, left, right, control, _]) = poseidon.lookup_poseidon_table();
        self.add_lookup(
            name,
            extended_queries,
            [
                q_enable.current(),
                hash.current(),
                left.current(),
                right.current(),
                control.current(),
            ],
        )
    }
}

#[derive(Clone, Copy)]
pub struct PoseidonTable {
    pub(crate) q_enable: FixedColumn,
    pub(crate) left: AdviceColumn,
    pub(crate) right: AdviceColumn,
    pub(crate) hash: AdviceColumn,
    pub(crate) control: AdviceColumn,
    pub(crate) domain: AdviceColumn,
    pub(crate) head_mark: AdviceColumn,
}

impl PoseidonTable {
    pub fn configure<F: Field>(cs: &mut ConstraintSystem<F>) -> Self {
        let [hash, left, right, control, domain, head_mark] =
            [0; 6].map(|_| AdviceColumn(cs.advice_column()));
        Self {
            left,
            right,
            hash,
            control,
            domain,
            head_mark,
            q_enable: FixedColumn(cs.fixed_column()),
        }
    }

    #[cfg(test)]
    pub fn load<'a, F: Field, I: Iterator<Item = &'a [F; 3]>>(
        &self,
        region: &mut Region<'_, F>,
        hash_traces: I,
    ) {
        for (offset, hash_trace) in hash_traces.enumerate() {
            assert_eq!(
                poseidon_hash(hash_trace[0], hash_trace[1]),
                hash_trace[2],
                "{:?}",
                (hash_trace[0], hash_trace[1], hash_trace[2])
            );
            for (column, value) in [
                (self.left, hash_trace[0]),
                (self.right, hash_trace[1]),
                (self.hash, hash_trace[2]),
                (self.control, F::zero()),
                (self.head_mark, F::one()),
            ] {
                column.assign(region, offset, value);
            }
            self.q_enable.assign(region, offset, F::one());
        }
    }

    pub fn table_columns(&self) -> (Column<Fixed>, [Column<Advice>; 6]) {
        (
            self.q_enable.0,
            [
                self.hash.0,
                self.left.0,
                self.right.0,
                self.control.0,
                self.domain.0,
                self.head_mark.0,
            ],
        )
    }
}

impl PoseidonLookup for PoseidonTable {
    fn lookup_poseidon_table(&self) -> (FixedColumn, [AdviceColumn; N_POSEIDON_LOOKUP_TABLE]) {
        (
            self.q_enable,
            [
                self.hash,
                self.left,
                self.right,
                self.control,
                self.head_mark,
            ],
        )
    }
}

#[derive(Clone)]
pub struct PoseidonCircuitConfig<F: Field> {
    poseidon_config: PoseidonHashConfig<F>,
    poseidon_table: PoseidonTable,
}

impl<F: Field> PoseidonCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, poseidon_table: PoseidonTable) -> Self {
        let poseidon_config = PoseidonHashConfig::configure_sub(
            cs,
            poseidon_table.table_columns(),
            HASH_BLOCK_STEP_SIZE,
        );
        Self {
            poseidon_config,
            poseidon_table,
        }
    }

    pub fn assign_bytecode(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledBytecode<F>,
    ) -> Result<(), Error> {
        let mut poseidon_hash_table = PoseidonHashTable::default();
        let hash_traces = bytecode.hash_traces();
        let code_hash = bytecode.code_hash();
        poseidon_hash_table.stream_inputs_with_check(
            &hash_traces,
            Some(code_hash),
            bytecode.len() as u64,
            HASH_BLOCK_STEP_SIZE,
        );
        let poseidon_hash_chip = PoseidonHashChip::<'_, F, { HASH_BLOCK_STEP_SIZE }>::construct(
            self.poseidon_config.clone(),
            &poseidon_hash_table,
            hash_traces.len() + 1,
        );
        poseidon_hash_chip.load(layouter)?;
        Ok(())
    }
}

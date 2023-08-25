#[cfg(test)]
use crate::util::hash as poseidon_hash;
use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, FixedColumn, Query},
    util::Field,
};
use halo2_proofs::plonk::{Advice, Column, Fixed};
#[cfg(test)]
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};

/// Lookup  represent the poseidon table in zkevm circuit
pub trait PoseidonLookup {
    fn lookup_columns(&self) -> (FixedColumn, [AdviceColumn; 5]) {
        let (fixed, adv) = self.lookup_columns_generic();
        (FixedColumn(fixed), adv.map(AdviceColumn))
    }
    fn lookup_columns_generic(&self) -> (Column<Fixed>, [Column<Advice>; 5]) {
        let (fixed, adv) = self.lookup_columns();
        (fixed.0, adv.map(|col| col.0))
    }
}

impl<F: Field> ConstraintBuilder<F> {
    pub fn poseidon_lookup(
        &mut self,
        name: &'static str,
        code_hash: Query<F>,
        left_input: Query<F>,
        right_input: Query<F>,
        poseidon: &impl PoseidonLookup,
    ) {
        let extended_queries = [
            Query::one(),        // selector
            code_hash,           // hash
            left_input.clone(),  // left
            right_input.clone(), // right
            Query::zero(),       // control
            Query::one(),        // head mark */
        ];

        let (q_enable, [hash, left, right, control, head_mark]) = poseidon.lookup_columns();

        self.add_lookup(
            name,
            extended_queries,
            [
                q_enable.current(),
                hash.current(),
                left.current(),
                right.current(),
                control.current(),
                head_mark.current(),
            ],
        )
    }

    pub fn poseidon_lookup_input(
        &mut self,
        name: &'static str,
        code_hash: Query<F>,
        input: Query<F>,
        is_left: bool,
        poseidon: &impl PoseidonLookup,
    ) {
        let extended_queries = [
            Query::one(),      // selector
            code_hash.clone(), // code hash
            input.clone(),     //input
            Query::zero(),     // control
            Query::one(),      // head mark
        ];

        let (q_enable, [hash, left, right, control, head_mark]) = poseidon.lookup_columns();

        self.add_lookup(
            name,
            extended_queries,
            [
                q_enable.current(),
                hash.current(),
                if is_left {
                    left.current()
                } else {
                    right.current()
                },
                control.current(),
                head_mark.current(),
            ],
        )
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub struct PoseidonTable {
    pub(crate) q_enable: FixedColumn,
    pub(crate) left: AdviceColumn,
    pub(crate) right: AdviceColumn,
    pub(crate) hash: AdviceColumn,
    pub(crate) control: AdviceColumn,
    pub(crate) head_mark: AdviceColumn,
}

#[cfg(test)]
impl PoseidonTable {
    pub fn configure<F: Field>(cs: &mut ConstraintSystem<F>) -> Self {
        let [hash, left, right, control, head_mark] =
            [0; 5].map(|_| AdviceColumn(cs.advice_column()));
        Self {
            left,
            right,
            hash,
            control,
            head_mark,
            q_enable: FixedColumn(cs.fixed_column()),
        }
    }

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

    pub fn table_columns(&self) -> (Column<Fixed>, [Column<Advice>; 5]) {
        (
            self.q_enable.0,
            [
                self.hash.0,
                self.left.0,
                self.right.0,
                self.control.0,
                self.head_mark.0,
            ],
        )
    }
}

#[cfg(test)]
impl PoseidonLookup for PoseidonTable {
    fn lookup_columns(&self) -> (FixedColumn, [AdviceColumn; 5]) {
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

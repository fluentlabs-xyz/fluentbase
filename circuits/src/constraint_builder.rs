use halo2_proofs::plonk::{ConstraintSystem, SecondPhase};

mod binary_column;
mod binary_query;
mod column;
mod query;

use crate::util::Field;
pub use binary_column::BinaryColumn;
pub use binary_query::BinaryQuery;
pub use column::{AdviceColumn, AdviceColumnPhase2, FixedColumn, SelectorColumn};
pub use query::{Query, ToExpr};

pub struct ConstraintBuilder<F: Field> {
    constraints: Vec<(&'static str, Query<F>)>,
    #[allow(clippy::type_complexity)]
    lookups: Vec<(&'static str, Vec<(Query<F>, Query<F>)>)>,

    conditions: Vec<BinaryQuery<F>>,
}

impl<F: Field> ConstraintBuilder<F> {
    pub fn new(every_row: SelectorColumn) -> Self {
        Self {
            constraints: vec![],
            lookups: vec![],

            conditions: vec![every_row.current()],
        }
    }

    pub fn every_row_selector(&self) -> BinaryQuery<F> {
        self.conditions
            .first()
            .expect("every_row selector should always be first condition")
            .clone()
    }

    pub fn assert_zero(&mut self, name: &'static str, query: Query<F>) {
        let condition = self
            .conditions
            .iter()
            .fold(BinaryQuery::one(), |a, b| a.and(b.clone()));
        self.constraints.push((name, condition.condition(query)))
    }

    pub fn assert_equal(&mut self, name: &'static str, left: Query<F>, right: Query<F>) {
        self.assert_zero(name, left - right)
    }

    pub fn assert(&mut self, name: &'static str, condition: BinaryQuery<F>) {
        self.assert_zero(name, Query::one() - condition);
    }

    pub fn assert_unreachable(&mut self, name: &'static str) {
        self.assert(name, BinaryQuery::zero());
    }

    pub fn condition(&mut self, condition: BinaryQuery<F>, configure: impl FnOnce(&mut Self)) {
        self.conditions.push(condition);
        configure(self);
        self.conditions.pop().unwrap();
    }

    pub fn enter_condition(&mut self, condition: BinaryQuery<F>) {
        self.conditions.push(condition);
    }

    pub fn leave_condition(&mut self) {
        self.conditions.pop().unwrap();
    }

    pub fn add_lookup<const N: usize>(
        &mut self,
        name: &'static str,
        left: [Query<F>; N],
        right: [Query<F>; N],
    ) {
        let condition = self
            .conditions
            .iter()
            .skip(1) // Save a degree by skipping every row selector
            .fold(BinaryQuery::one(), |a, b| a.and(b.clone()));
        let lookup = left
            .into_iter()
            .map(|q| q * condition.clone())
            .zip(right.into_iter())
            .collect();
        self.lookups.push((name, lookup))
    }

    pub fn add_lookup_with_default<const N: usize>(
        &mut self,
        name: &'static str,
        left: [Query<F>; N],
        right: [Query<F>; N],
        default: [Query<F>; N],
    ) {
        let condition = self
            .conditions
            .iter()
            .skip(1) // Save a degree by skipping every row selector
            .fold(BinaryQuery::one(), |a, b| a.and(b.clone()));
        let lookup = left
            .into_iter()
            .zip(default.into_iter())
            .map(|(a, b)| condition.select(a, b))
            .zip(right.into_iter())
            .collect();
        self.lookups.push((name, lookup))
    }

    pub fn build_columns<const A: usize, const B: usize, const C: usize>(
        &self,
        cs: &mut ConstraintSystem<F>,
    ) -> ([SelectorColumn; A], [FixedColumn; B], [AdviceColumn; C]) {
        let selectors = [0; A].map(|_| SelectorColumn(cs.fixed_column()));
        let fixed_columns = [0; B].map(|_| FixedColumn(cs.fixed_column()));
        let advice_columns = [0; C].map(|_| AdviceColumn(cs.advice_column()));
        (selectors, fixed_columns, advice_columns)
    }

    pub fn advice_columns<const N: usize>(
        &self,
        cs: &mut ConstraintSystem<F>,
    ) -> [AdviceColumn; N] {
        [0; N].map(|_| AdviceColumn(cs.advice_column()))
    }

    pub fn advice_column(&self, cs: &mut ConstraintSystem<F>) -> AdviceColumn {
        AdviceColumn(cs.advice_column())
    }

    pub fn advice_column_phase2(&self, cs: &mut ConstraintSystem<F>) -> AdviceColumnPhase2 {
        AdviceColumnPhase2(cs.advice_column_in(SecondPhase))
    }

    pub fn fixed_columns<const N: usize>(&self, cs: &mut ConstraintSystem<F>) -> [FixedColumn; N] {
        [0; N].map(|_| FixedColumn(cs.fixed_column()))
    }

    pub fn fixed_column(&self, cs: &mut ConstraintSystem<F>) -> FixedColumn {
        FixedColumn(cs.fixed_column())
    }

    pub fn second_phase_advice_columns<const N: usize>(
        &self,
        cs: &mut ConstraintSystem<F>,
    ) -> [AdviceColumnPhase2; N] {
        [0; N].map(|_| AdviceColumnPhase2(cs.advice_column_in(SecondPhase)))
    }

    pub fn binary_columns<const N: usize>(
        &mut self,
        cs: &mut ConstraintSystem<F>,
    ) -> [BinaryColumn; N] {
        [0; N].map(|_| BinaryColumn::configure::<F>(cs, self))
    }

    pub fn build(&self, cs: &mut ConstraintSystem<F>) {
        assert_eq!(
            self.conditions.len(),
            1,
            "Cannot call build while in a condition"
        );

        for (name, query) in &self.constraints {
            cs.create_gate(name, |meta| vec![query.run(meta)])
        }
        for (name, lookup) in &self.lookups {
            cs.lookup_any(name, |meta| {
                lookup
                    .into_iter()
                    .map(|(left, right)| (left.run(meta), right.run(meta)))
                    .collect()
            });
        }
    }
}

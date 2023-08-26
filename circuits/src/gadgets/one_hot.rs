use crate::{
    constraint_builder::{BinaryColumn, BinaryQuery, ConstraintBuilder, Query},
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::{cmp::Eq, collections::BTreeMap};
use strum::IntoEnumIterator;

// One hot encoding for an enum with T::COUNT variants with COUNT - 1 binary columns.
// It's useful to have 1 less column so that the default assigment for the gadget
// is valid (it will represent the first variant).
#[derive(Clone)]
pub struct OneHot<T: PartialOrd + Ord> {
    columns: BTreeMap<T, BinaryColumn>,
}

impl<T: IntoEnumIterator + Eq + PartialOrd + Ord> OneHot<T> {
    pub fn configure<F: Field>(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
    ) -> Self {
        let mut columns = BTreeMap::new();
        for variant in Self::nonfirst_variants() {
            columns.insert(variant, cb.binary_columns::<1>(cs)[0]);
        }
        let config = Self { columns };
        cb.assert(
            "sum of binary columns in OneHot is 0 or 1",
            config.sum(0).or(!config.sum(0)),
        );
        config
    }

    pub fn assign<F: Field>(&self, region: &mut Region<'_, F>, offset: usize, value: T) {
        if let Some(c) = self.columns.get(&value) {
            c.assign(region, offset, true)
        }
    }

    pub fn previous_matches<F: Field>(&self, values: &[T]) -> BinaryQuery<F> {
        self.matches(values, -1)
    }

    pub fn current_matches<F: Field>(&self, values: &[T]) -> BinaryQuery<F> {
        self.matches(values, 0)
    }

    pub fn next_matches<F: Field>(&self, values: &[T]) -> BinaryQuery<F> {
        self.matches(values, 1)
    }

    fn matches<F: Field>(&self, values: &[T], r: i32) -> BinaryQuery<F> {
        let query = values
            .iter()
            .map(|v| {
                self.columns
                    .get(v)
                    .map_or_else(|| !self.sum(r), |c| c.rotation(r))
            })
            .fold(Query::zero(), |a, b| a + b);
        // This cast is ok (if the values are distinct) because at most one column is set.
        BinaryQuery(query)
    }

    pub fn current<F: Field>(&self) -> Query<F> {
        T::iter().enumerate().fold(Query::zero(), |acc, (i, t)| {
            acc + Query::from(u64::try_from(i).unwrap())
                * self
                    .columns
                    .get(&t)
                    .map_or_else(|| !self.sum(0), BinaryColumn::current)
        })
    }

    pub fn previous<F: Field>(&self) -> Query<F> {
        T::iter().enumerate().fold(Query::zero(), |acc, (i, t)| {
            acc + Query::from(u64::try_from(i).unwrap())
                * self
                    .columns
                    .get(&t)
                    .map_or_else(|| !self.sum(-1), BinaryColumn::current)
        })
    }

    fn sum<F: Field>(&self, r: i32) -> BinaryQuery<F> {
        BinaryQuery(
            self.columns
                .values()
                .fold(Query::zero(), |a: Query<F>, b| a + b.rotation(r)),
        )
    }

    fn nonfirst_variants() -> impl Iterator<Item = T> {
        let mut variants = T::iter();
        variants.next();
        variants
    }
}

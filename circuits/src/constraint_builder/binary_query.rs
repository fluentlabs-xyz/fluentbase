use super::Query;
use halo2_proofs::{
    arithmetic::{Field, FieldExt},
    plonk::{Expression, VirtualCells},
};
// use std::iter::Sum;

/// A query whose expression we promise is 0 or 1.
#[derive(Clone)]
pub struct BinaryQuery<F: Field>(pub Query<F>);

impl<F: FieldExt> BinaryQuery<F> {
    pub fn zero() -> Self {
        Self(Query::from(0))
    }

    pub fn one() -> Self {
        Self(Query::from(1))
    }

    pub fn and(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }

    pub fn or(self, other: Self) -> Self {
        !((!self).and(!other))
    }

    pub fn condition(self, constraint: Query<F>) -> Query<F> {
        self.0 * constraint
    }

    pub fn select(&self, if_true: Query<F>, if_false: Query<F>) -> Query<F> {
        if_true * self.clone() + if_false * !self.clone()
    }
}

impl<F: FieldExt> BinaryQuery<F> {
    pub fn run(self, meta: &mut VirtualCells<'_, F>) -> Expression<F> {
        self.0.run(meta)
    }
}

impl<F: FieldExt> std::ops::Not for BinaryQuery<F> {
    type Output = Self;

    // In general this can cause a ConstraintPoisoned. You need to add a selector column that's all ones to be safe.
    fn not(self) -> Self::Output {
        Self(Query::one() - self.0)
    }
}

use crate::{
    constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query},
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::fmt::Debug;

#[derive(Clone)]
pub struct IsZeroConfig<F: Field> {
    pub value: Query<F>,
    pub inverse_or_zero: AdviceColumn,
}

impl<F: Field> IsZeroConfig<F> {
    pub fn current(self) -> BinaryQuery<F> {
        BinaryQuery(Query::one() - self.value * self.inverse_or_zero.current())
    }

    pub fn assign<T: Copy + TryInto<F>>(&self, region: &mut Region<'_, F>, offset: usize, value: T)
    where
        <T as TryInto<F>>::Error: Debug,
    {
        self.inverse_or_zero.assign(
            region,
            offset,
            value.try_into().unwrap().invert().unwrap_or(F::zero()),
        );
    }

    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
        value: Query<F>,
    ) -> Self {
        let inverse_or_zero = AdviceColumn(cs.advice_column());
        cb.assert_zero(
            "value is 0 or inverse_or_zero is inverse of value",
            value.clone() * (Query::one() - value.clone() * inverse_or_zero.current()),
        );
        Self {
            value,
            inverse_or_zero,
        }
    }
}

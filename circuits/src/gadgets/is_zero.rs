use crate::constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query};
use halo2_proofs::{arithmetic::FieldExt, circuit::Region, plonk::ConstraintSystem};
use std::fmt::Debug;

#[derive(Clone, Copy)]
pub struct IsZeroGadget {
    pub value: AdviceColumn,
    pub inverse_or_zero: AdviceColumn,
}

impl IsZeroGadget {
    pub fn current<F: FieldExt>(self) -> BinaryQuery<F> {
        BinaryQuery(Query::one() - self.value.current() * self.inverse_or_zero.current())
    }

    pub fn previous<F: FieldExt>(self) -> BinaryQuery<F> {
        BinaryQuery(Query::one() - self.value.previous() * self.inverse_or_zero.previous())
    }

    pub fn assign<F: FieldExt, T: Copy + TryInto<F>>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: T,
    ) where
        <T as TryInto<F>>::Error: Debug,
    {
        self.inverse_or_zero.assign(
            region,
            offset,
            value.try_into().unwrap().invert().unwrap_or(F::zero()),
        );
    }

    // TODO: get rid of assign method in favor of it.
    pub fn assign_value_and_inverse<F: FieldExt, T: Copy + TryInto<F>>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: T,
    ) where
        <T as TryInto<F>>::Error: Debug,
    {
        self.value.assign(region, offset, value);
        self.assign(region, offset, value);
    }

    pub fn configure<F: FieldExt>(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
        value: AdviceColumn, // TODO: make this a query once Query is clonable/copyable.....
    ) -> Self {
        let inverse_or_zero = AdviceColumn(cs.advice_column());
        cb.assert_zero(
            "value is 0 or inverse_or_zero is inverse of value",
            value.current() * (Query::one() - value.current() * inverse_or_zero.current()),
        );
        Self {
            value,
            inverse_or_zero,
        }
    }
}

use crate::{
    constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query, ToExpr},
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::fmt::Debug;
use crate::runtime_circuit::constraint_builder::OpConstraintBuilder;

#[derive(Clone)]
pub struct F32ExpConfig<F: Field> {
    pub value: AdviceColumn,
    pub denorm_case_inv: AdviceColumn,
    pub inf_case_inv: AdviceColumn,
    pub _marker: std::marker::PhantomData<F>,
}

impl<F: Field> F32ExpConfig<F> {

    pub fn current(&self) -> Query<F> {
        self.value.current()
    }

    pub fn is_norm(&self) -> BinaryQuery<F> {
        BinaryQuery(
            self.value.current() * self.denorm_case_inv.current()
            + (self.value.current() - 255.expr()) * self.inf_case_inv.current()
            - Query::one()
        )
    }

    pub fn is_denorm(&self) -> BinaryQuery<F> {
        BinaryQuery(Query::one() - self.value.current() * self.denorm_case_inv.current())
    }

    pub fn is_inf_or_nan(&self) -> BinaryQuery<F> {
        BinaryQuery(Query::one() - (self.value.current() - 255.expr()) * self.inf_case_inv.current())
    }

    pub fn assign<T: Copy + TryInto<F>>(&self, region: &mut Region<'_, F>, offset: usize, value: T)
    where
        <T as TryInto<F>>::Error: Debug,
    {
        self.value.assign(
            region,
            offset,
            value.try_into().unwrap(),
        );
        self.denorm_case_inv.assign(
            region,
            offset,
            value.try_into().unwrap().invert().unwrap_or(F::zero()),
        );
        self.inf_case_inv.assign(
            region,
            offset,
            (value.try_into().unwrap() - F::from(255_u64)).invert().unwrap_or(F::zero()),
        );
    }

    pub fn configure(
        ocb: &mut OpConstraintBuilder<F>,
    ) -> Self {
        let value = ocb.query_cell();
        let denorm_case_inv = ocb.query_cell();
        let inf_case_inv = ocb.query_cell();
        ocb.range_check8(value.current());
        let cb = &mut ocb.base;
        // TODO: also perform range check.
        cb.assert_zero(
            "value can be 0, 255, or between",
            value.current()
              * (Query::one() - value.current() * denorm_case_inv.current())
              * (Query::one() - value.current() * inf_case_inv.current()),
        );
        Self {
            value,
            denorm_case_inv,
            inf_case_inv,
            _marker: std::marker::PhantomData,
        }
    }
}

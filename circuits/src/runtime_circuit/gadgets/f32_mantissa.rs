use crate::{
    constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query, ToExpr},
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::fmt::Debug;
use crate::runtime_circuit::constraint_builder::OpConstraintBuilder;

#[derive(Clone)]
pub struct F32MantissaConfig<F: Field> {
    pub limbs: [AdviceColumn; 3],
    pub _marker: std::marker::PhantomData<F>,
}

impl<F: Field> F32MantissaConfig<F> {

    pub fn absolute(&self) -> Query<F> {
        self.limbs[2].current() * 0x10000.expr() + self.limbs[1].current() * 0x100.expr() + self.limbs[0].current()
    }

    pub fn raw_part(&self) -> Query<F> {
        (self.limbs[2].current() - 0x80.expr()) * 0x10000.expr() + self.limbs[1].current() * 0x100.expr() + self.limbs[0].current()
    }

    pub fn configure(
        cb: &mut OpConstraintBuilder<F>,
    ) -> Self {
        let limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        for i in 0..2 {
            cb.range_check8(limbs[i].current());
        }
        cb.range_check7(limbs[2].current() - 0x80.expr());
        Self {
            limbs,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn assign_from_raw(&self, region: &mut Region<'_, F>, offset: usize, value: u32) -> u32
    {
        // Here in last limb, extra bit is added, bit number 24 in mantissa that always one.
        // TODO: case with un normalized form.
        let limbs = [value & 0xff, (value >> 8) & 0xff, ((value >> 16) & 0x7f) | 0x80];
        for i in 0..=2 {
            self.limbs[i].assign(region, offset, F::from(limbs[i] as u64));
        }
        (value & 0x7fffff) | 0x800000
    }

}

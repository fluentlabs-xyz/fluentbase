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
    pub inverse_or_zero: AdviceColumn,
    pub _marker: std::marker::PhantomData<F>,
}

impl<F: Field> F32MantissaConfig<F> {

    pub fn is_zero(&self, is_extra_bit: Query<F>) -> BinaryQuery<F> {
        BinaryQuery(1.expr() - self.raw_part(is_extra_bit) * self.inverse_or_zero.current())
    }

    pub fn absolute(&self) -> Query<F> {
        self.limbs[2].current() * 0x10000.expr() + self.limbs[1].current() * 0x100.expr() + self.limbs[0].current()
    }

    pub fn raw_part(&self, is_extra_bit: Query<F>) -> Query<F> {
        (self.limbs[2].current() - 0x80.expr() * is_extra_bit) * 0x10000.expr()
            + self.limbs[1].current() * 0x100.expr()
            + self.limbs[0].current()
    }

    pub fn configure(
        cb: &mut OpConstraintBuilder<F>,
        is_extra_bit: Query<F>,
    ) -> Self {
        let limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        let inverse_or_zero = cb.query_cell();
        for i in 0..2 {
            cb.range_check8(limbs[i].current());
        }
        cb.range_check7(limbs[2].current() - is_extra_bit.clone() * 0x80.expr());
        let this = Self {
            limbs,
            inverse_or_zero,
            _marker: std::marker::PhantomData,
        };
        cb.require_zero(
            "raw_part is 0 or inverse_or_zero is inverse of value",
            this.raw_part(is_extra_bit.clone()) * (1.expr() - this.raw_part(is_extra_bit) * this.inverse_or_zero.current()),
        );
        this
    }

    pub fn assign_from_raw(&self, region: &mut Region<'_, F>, offset: usize, value: u32) -> (u32, u32)
    {
        let is_extra_bit: u32 = if (value >> 23) & 0xff == 0 { 0 } else { 1 };
        // Here in last limb, extra bit is added, bit number 24 in mantissa that always one.
        // TODO: case with un normalized form.
        let limbs = [value & 0xff, (value >> 8) & 0xff, ((value >> 16) & 0x7f) | is_extra_bit * 0x80];
        for i in 0..=2 {
            self.limbs[i].assign(region, offset, F::from(limbs[i] as u64));
        }
        let raw_part = value & 0x7fffff;
        self.inverse_or_zero.assign(
            region,
            offset,
            F::from(raw_part as u64).invert().unwrap_or(F::zero()),
        );
        (raw_part, raw_part | is_extra_bit * 0x800000)
    }

}

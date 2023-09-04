use crate::{
    constraint_builder::{FixedColumn, Query},
    lookup_table::{RangeCheckLookup, N_RANGE_CHECK_LOOKUP_TABLE},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct RangeCheckConfig<F: Field> {
    u8: FixedColumn,
    u10: FixedColumn,
    u16: FixedColumn,
    marker: PhantomData<F>,
}

impl<F: Field> RangeCheckConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            u8: FixedColumn(cs.fixed_column()),
            u10: FixedColumn(cs.fixed_column()),
            u16: FixedColumn(cs.fixed_column()),
            marker: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "range check table",
            |mut region| {
                let ranges = [
                    (&self.u8, 0..=0x00ff),
                    (&self.u10, 0..=0x03ff),
                    // (&self.u16, 0..=0xffff),
                ];
                for (col, range) in ranges {
                    for b in range {
                        col.assign(&mut region, b, b as u64);
                    }
                }
                Ok(())
            },
        )
    }
}

impl<F: Field> RangeCheckLookup<F> for RangeCheckConfig<F> {
    fn lookup_u8_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE] {
        [self.u8.current()]
    }

    fn lookup_u10_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE] {
        [self.u10.current()]
    }

    fn lookup_u16_table(&self) -> [Query<F>; N_RANGE_CHECK_LOOKUP_TABLE] {
        [self.u16.current()]
    }
}

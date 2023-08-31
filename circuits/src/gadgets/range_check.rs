use crate::{
    constraint_builder::{FixedColumn, Query},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

pub trait RangeCheckLookup<F: Field> {
    fn lookup_u8_table(&self) -> [Query<F>; 1];

    fn lookup_u16_table(&self) -> [Query<F>; 1];
}

#[derive(Clone)]
pub struct RangeCheckConfig<F: Field> {
    u8: FixedColumn,
    u16: FixedColumn,
    marker: PhantomData<F>,
}

impl<F: Field> RangeCheckConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            u8: FixedColumn(cs.fixed_column()),
            u16: FixedColumn(cs.fixed_column()),
            marker: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "range check table",
            |mut region| {
                for b0 in 0..=0xff {
                    self.u8.assign(&mut region, b0, b0 as u64);
                    for b1 in 0..=0xff {
                        let offset = b0 * 0x100 + b1;
                        self.u16.assign(&mut region, offset, offset as u64);
                    }
                }
                Ok(())
            },
        )
    }
}

impl<F: Field> RangeCheckLookup<F> for RangeCheckConfig<F> {
    fn lookup_u8_table(&self) -> [Query<F>; 1] {
        [self.u8.current()]
    }

    fn lookup_u16_table(&self) -> [Query<F>; 1] {
        [self.u16.current()]
    }
}

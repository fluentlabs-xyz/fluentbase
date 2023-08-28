use super::super::constraint_builder::{ConstraintBuilder, FixedColumn, Query};
use crate::util::Field;
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};

#[derive(Clone)]
pub struct ByteBitConfig {
    byte: FixedColumn,
    index: FixedColumn,
    bit: FixedColumn,
}

pub trait RangeCheck8Lookup {
    fn lookup<F: Field>(&self) -> [Query<F>; 1];
}

pub trait RangeCheck256Lookup {
    fn lookup<F: Field>(&self) -> [Query<F>; 1];
}

pub trait ByteBitLookup {
    fn lookup<F: Field>(&self) -> [Query<F>; 3];
}

impl ByteBitConfig {
    pub fn configure<F: Field>(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
    ) -> Self {
        let ([], [byte, index, bit], []) = cb.build_columns(cs);
        Self { byte, index, bit }
    }

    pub fn assign<F: Field>(&self, region: &mut Region<'_, F>) {
        let mut offset = 0;
        for byte in 0..256 {
            for index in 0..8 {
                self.byte.assign(region, offset, byte);
                self.index.assign(region, offset, index);
                self.bit.assign(region, offset, byte & (1 << index) != 0);
                offset += 1;
            }
        }
    }
}

impl RangeCheck8Lookup for ByteBitConfig {
    fn lookup<F: Field>(&self) -> [Query<F>; 1] {
        [self.index.current()]
    }
}

impl RangeCheck256Lookup for ByteBitConfig {
    fn lookup<F: Field>(&self) -> [Query<F>; 1] {
        [self.byte.current()]
    }
}

impl ByteBitLookup for ByteBitConfig {
    fn lookup<F: Field>(&self) -> [Query<F>; 3] {
        [
            self.byte.current(),
            self.index.current(),
            self.bit.current(),
        ]
    }
}

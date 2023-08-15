use super::super::constraint_builder::{ConstraintBuilder, FixedColumn, Query};
use halo2_proofs::{arithmetic::FieldExt, circuit::Region, plonk::ConstraintSystem};

// TODO: fix name to configggggggg
#[derive(Clone)]
pub struct ByteBitGadget {
    byte: FixedColumn,
    index: FixedColumn,
    bit: FixedColumn,
}

pub trait RangeCheck8Lookup {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 1];
}

pub trait RangeCheck256Lookup {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 1];
}

pub trait ByteBitLookup {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 3];
}

impl ByteBitGadget {
    pub fn configure<F: FieldExt>(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
    ) -> Self {
        let ([], [byte, index, bit], []) = cb.build_columns(cs);
        Self { byte, index, bit }
    }

    pub fn assign<F: FieldExt>(&self, region: &mut Region<'_, F>) {
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

impl RangeCheck8Lookup for ByteBitGadget {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 1] {
        [self.index.current()]
    }
}

impl RangeCheck256Lookup for ByteBitGadget {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 1] {
        [self.byte.current()]
    }
}

impl ByteBitLookup for ByteBitGadget {
    fn lookup<F: FieldExt>(&self) -> [Query<F>; 3] {
        [
            self.byte.current(),
            self.index.current(),
            self.bit.current(),
        ]
    }
}

use crate::{
    constraint_builder::{FixedColumn, Query},
    lookup_table::{BitwiseCheckLookup, N_BITWISE_CHECK_LOOKUP_TABLE},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use log::debug;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct BitwiseCheckConfig<F: Field> {
    and: [FixedColumn; N_BITWISE_CHECK_LOOKUP_TABLE],
    or: [FixedColumn; N_BITWISE_CHECK_LOOKUP_TABLE],
    xor: [FixedColumn; N_BITWISE_CHECK_LOOKUP_TABLE],
    _marker: PhantomData<F>,
}

impl<F: Field> BitwiseCheckConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            and: [0; N_BITWISE_CHECK_LOOKUP_TABLE].map(|v| FixedColumn(cs.fixed_column())),
            or: [0; N_BITWISE_CHECK_LOOKUP_TABLE].map(|v| FixedColumn(cs.fixed_column())),
            xor: [0; N_BITWISE_CHECK_LOOKUP_TABLE].map(|v| FixedColumn(cs.fixed_column())),
            _marker: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "bitwise check table",
            |mut region| {
                const MAX_VAL: u64 = 256;
                let mut offset = 0;
                (0..MAX_VAL).for_each(|lhs| {
                    (0..MAX_VAL).for_each(|rhs| {
                        let and = lhs & rhs;
                        let or = lhs | rhs;
                        let xor = lhs ^ rhs;
                        debug!(
                            "bitwise check table: assign at offset {} lhs {} rhs {} and {} or {} xor {}",
                            offset, lhs, rhs, and, or, xor
                        );

                        self.and[0].assign(&mut region, offset, lhs);
                        self.and[1].assign(&mut region, offset, rhs);
                        self.and[2].assign(&mut region, offset, and);

                        self.or[0].assign(&mut region, offset, lhs);
                        self.or[1].assign(&mut region, offset, rhs);
                        self.or[2].assign(&mut region, offset, or);

                        self.xor[0].assign(&mut region, offset, lhs);
                        self.xor[1].assign(&mut region, offset, rhs);
                        self.xor[2].assign(&mut region, offset, xor);

                        offset += 1;
                    })
                });

                Ok(())
            },
        )
    }
}

impl<F: Field> BitwiseCheckLookup<F> for BitwiseCheckConfig<F> {
    fn lookup_and(&self) -> [Query<F>; N_BITWISE_CHECK_LOOKUP_TABLE] {
        [self.and[0], self.and[1], self.and[2]].map(|v| v.current())
    }

    fn lookup_or(&self) -> [Query<F>; N_BITWISE_CHECK_LOOKUP_TABLE] {
        [self.or[0], self.or[1], self.or[2]].map(|v| v.current())
    }

    fn lookup_xor(&self) -> [Query<F>; N_BITWISE_CHECK_LOOKUP_TABLE] {
        [self.xor[0], self.xor[1], self.xor[2]].map(|v| v.current())
    }
}

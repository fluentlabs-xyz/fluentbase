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
    inputs: [FixedColumn; 2],
    and: [FixedColumn; 1],
    or: [FixedColumn; 1],
    xor: [FixedColumn; 1],
    _marker: PhantomData<F>,
}

impl<F: Field> BitwiseCheckConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            inputs: [0; 2].map(|v| FixedColumn(cs.fixed_column())),
            and: [0; 1].map(|v| FixedColumn(cs.fixed_column())),
            or: [0; 1].map(|v| FixedColumn(cs.fixed_column())),
            xor: [0; 1].map(|v| FixedColumn(cs.fixed_column())),
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

                        self.inputs[0].assign(&mut region, offset, lhs);
                        self.inputs[1].assign(&mut region, offset, rhs);

                        self.and[0].assign(&mut region, offset, and);

                        self.or[0].assign(&mut region, offset, or);

                        self.xor[0].assign(&mut region, offset, xor);

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
        [self.inputs[0], self.inputs[1], self.and[0]].map(|v| v.current())
    }

    fn lookup_or(&self) -> [Query<F>; N_BITWISE_CHECK_LOOKUP_TABLE] {
        [self.inputs[0], self.inputs[1], self.or[0]].map(|v| v.current())
    }

    fn lookup_xor(&self) -> [Query<F>; N_BITWISE_CHECK_LOOKUP_TABLE] {
        [self.inputs[0], self.inputs[1], self.xor[0]].map(|v| v.current())
    }
}

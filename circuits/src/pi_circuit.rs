use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, InstanceColumn, Query, SelectorColumn},
    lookup_table::{PublicInputLookup, N_EXIT_CODE_LOOKUP_TABLE, N_PUBLIC_INPUT_LOOKUP_TABLE},
    poseidon_circuit::PoseidonLookup,
    util::Field,
    witness::{UnrolledPublicInput, N_PUBLIC_INPUT_BYTES},
};
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct PublicInputCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    instance: InstanceColumn,
    index: AdviceColumn,
    exit_code: AdviceColumn,
    input: AdviceColumn,
    output: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, _poseidon_lookup: &impl PoseidonLookup) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let cb = ConstraintBuilder::new(q_enable);

        let instance = cb.instance_column(cs);
        let index = cb.advice_column(cs);
        let input = cb.advice_column(cs);
        let exit_code = cb.advice_column(cs);
        let output = cb.advice_column(cs);

        cs.enable_equality(instance);
        cs.enable_equality(exit_code);
        cs.enable_equality(input);
        cs.enable_equality(output);

        cb.build(cs);
        Self {
            q_enable,
            instance,
            index,
            exit_code,
            input,
            output,
            marker: Default::default(),
        }
    }

    pub fn assign_with_region(
        &self,
        region: &mut Region<'_, F>,
        public_input: &UnrolledPublicInput<F>,
    ) -> Result<(), Error> {
        for (i, byte) in public_input.input().iter().enumerate() {
            self.input.assign(region, i, *byte as u64);
        }
        for (i, byte) in public_input.output().iter().enumerate() {
            self.output.assign(region, i, *byte as u64);
        }
        let max_rows = public_input.input().len().max(public_input.output().len());
        (0..max_rows).for_each(|offset| {
            self.q_enable.enable(region, offset);
        });
        let max_indices = (max_rows + N_PUBLIC_INPUT_BYTES - 1) / N_PUBLIC_INPUT_BYTES;
        (0..max_indices).for_each(|index| {
            self.index
                .assign(region, index / N_PUBLIC_INPUT_BYTES, index as u64);
        });
        Ok(())
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        public_input: &UnrolledPublicInput<F>,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "public input assign",
            |mut region| self.assign_with_region(&mut region, public_input),
        )?;
        Ok(())
    }

    pub fn expose_public(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "exit code instance",
            |mut region| {
                self.q_enable.enable(&mut region, 0);
                region.assign_advice_from_instance(
                    || "exit code instance",
                    self.instance.0,
                    0,
                    self.exit_code.0,
                    0,
                )?;
                region.assign_advice_from_instance(
                    || "input instance",
                    self.instance.0,
                    1,
                    self.input.0,
                    0,
                )?;
                region.assign_advice_from_instance(
                    || "output instance",
                    self.instance.0,
                    2,
                    self.output.0,
                    0,
                )?;
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: Field> PublicInputLookup<F> for PublicInputCircuitConfig<F> {
    fn lookup_input_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.index.current(),
            self.input.current(),
        ]
    }

    fn lookup_output_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.index.current(),
            self.output.current(),
        ]
    }

    fn lookup_exit_code(&self) -> [Query<F>; N_EXIT_CODE_LOOKUP_TABLE] {
        [self.q_enable.current().0, self.exit_code.current()]
    }
}

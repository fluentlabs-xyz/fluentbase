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
    input: AdviceColumn,
    private_input: AdviceColumn,
    output: AdviceColumn,
    private_output: AdviceColumn,
    exit_code: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, _poseidon_lookup: &impl PoseidonLookup) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let cb = ConstraintBuilder::new(q_enable);

        let instance = cb.instance_column(cs);
        let input = cb.advice_column(cs);
        let index = cb.advice_column(cs);
        let private_input = cb.advice_column(cs);
        let output = cb.advice_column(cs);
        let private_output = cb.advice_column(cs);
        let exit_code = cb.advice_column(cs);

        // let input_offset = cb.fixed_column(cs);
        // let output_offset = cb.fixed_column(cs);

        cs.enable_equality(instance);
        cs.enable_equality(exit_code);
        cs.enable_equality(input);
        cs.enable_equality(output);

        // cb.poseidon_lookup(
        //     "poseidon lookup public input",
        //     input.current(),
        //     private_input.current(),
        //     private_input.next(),
        //     input_offset.current(),
        //     poseidon_lookup,
        // );

        cb.build(cs);
        Self {
            q_enable,
            input,
            index,
            private_input,
            instance,
            output,
            private_output,
            exit_code,
            marker: Default::default(),
        }
    }

    pub fn assign_with_region(
        &self,
        region: &mut Region<'_, F>,
        public_input: &UnrolledPublicInput<F>,
    ) -> Result<(), Error> {
        for (i, word) in public_input.input().words().iter().enumerate() {
            self.private_input.assign(region, 2 * i, word[0]);
            self.private_input.assign(region, 2 * i + 1, word[1]);
        }
        for (i, word) in public_input.output().words().iter().enumerate() {
            self.private_output.assign(region, 2 * i, word[0]);
            self.private_output.assign(region, 2 * i + 1, word[1]);
        }
        let max_rows = public_input
            .input()
            .length()
            .max(public_input.output().length());
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
            self.private_input.current(),
        ]
    }

    fn lookup_output_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        [
            self.q_enable.current().0,
            self.index.current(),
            self.private_output.current(),
        ]
    }

    fn lookup_exit_code(&self) -> [Query<F>; N_EXIT_CODE_LOOKUP_TABLE] {
        [self.q_enable.current().0, self.exit_code.current()]
    }
}

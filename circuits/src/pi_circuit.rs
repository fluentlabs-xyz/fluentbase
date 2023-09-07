use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, InstanceColumn, Query, SelectorColumn},
    lookup_table::{PublicInputLookup, N_PUBLIC_INPUT_LOOKUP_TABLE},
    poseidon_circuit::PoseidonLookup,
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct PublicInputCircuitConfig<F: Field> {
    q_enable: SelectorColumn,
    instance: InstanceColumn,
    input: AdviceColumn,
    output: AdviceColumn,
    exit_code: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> PublicInputCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, _poseidon_lookup: &impl PoseidonLookup) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let cb = ConstraintBuilder::new(q_enable);
        let instance = cb.instance_column(cs);
        let input = cb.advice_column(cs);
        let output = cb.advice_column(cs);
        let exit_code = cb.advice_column(cs);
        cs.enable_equality(instance);
        // cs.enable_equality(input);
        // cs.enable_equality(output);
        cs.enable_equality(exit_code);
        cb.build(cs);
        Self {
            q_enable,
            input,
            instance,
            output,
            exit_code,
            marker: Default::default(),
        }
    }

    pub fn expose_public(
        &self,
        layouter: &mut impl Layouter<F>,
        _input: &Vec<u8>,
        _output: &Vec<u8>,
        _exit_code: i32,
    ) -> Result<(), Error> {
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
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: Field> PublicInputLookup<F> for PublicInputCircuitConfig<F> {
    fn lookup_input_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        todo!()
    }

    fn lookup_output_byte(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        todo!()
    }

    fn lookup_exit_code(&self) -> [Query<F>; N_PUBLIC_INPUT_LOOKUP_TABLE] {
        [self.q_enable.current().0, self.exit_code.current()]
    }
}

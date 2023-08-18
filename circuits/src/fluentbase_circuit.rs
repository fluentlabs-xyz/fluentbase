mod op_const;

use crate::{
    constraint_builder::{ConstraintBuilder, SelectorColumn},
    gadgets::poseidon::{PoseidonLookup, PoseidonTable},
    util::unroll_to_hash_input,
};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Chip, Layouter, Region},
    halo2curves::bn256::Fr,
    plonk::{ConstraintSystem, Error},
};
use hash_circuit::hash::{PoseidonHashChip, PoseidonHashConfig, PoseidonHashTable};

#[derive(Clone)]
pub struct FluentbaseCircuitConfig {
    poseidon_config: PoseidonHashConfig<Fr>,
    poseidon_table: PoseidonTable,
}

impl FluentbaseCircuitConfig {
    pub fn configure(cs: &mut ConstraintSystem<Fr>, cb: &mut ConstraintBuilder<Fr>) -> Self {
        let poseidon_table = PoseidonTable::configure(cs);
        let poseidon_config =
            PoseidonHashConfig::configure_sub(cs, poseidon_table.table_columns(), 32);
        // cb.poseidon_lookup("lookup", )
        Self {
            poseidon_config,
            poseidon_table,
        }
    }

    pub fn assign(&self, region: &mut Region<'_, Fr>, rwasm_binary: &Vec<u8>) -> Result<(), Error> {
        Ok(())
    }

    pub fn load(
        &self,
        layouter: &mut impl Layouter<Fr>,
        rwasm_binary: &Vec<u8>,
    ) -> Result<(), Error> {
        const STEP: usize = 31;
        let hash_input = unroll_to_hash_input::<Fr, STEP, 2>(rwasm_binary.iter().copied());
        let mut poseidon_hash_table = PoseidonHashTable::default();
        poseidon_hash_table.stream_inputs(&hash_input, rwasm_binary.len() as u64, STEP);
        let poseidon_hash_chip = PoseidonHashChip::<'_, Fr, STEP>::construct(
            self.poseidon_config.clone(),
            &poseidon_hash_table,
            rwasm_binary.len(),
        );
        poseidon_hash_chip.load(layouter)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halo2_proofs::{circuit::SimpleFloorPlanner, dev::MockProver, plonk::Circuit};

    #[derive(Clone, Default, Debug)]
    struct TestCircuit {
        rwasm_binary: Vec<u8>,
    }

    impl Circuit<Fr> for TestCircuit {
        type Config = (SelectorColumn, FluentbaseCircuitConfig);
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(cs: &mut ConstraintSystem<Fr>) -> Self::Config {
            let selector = SelectorColumn(cs.fixed_column());
            let mut cb = ConstraintBuilder::new(selector);
            let rwasm_runtime = FluentbaseCircuitConfig::configure(cs, &mut cb);
            cb.build(cs);
            (selector, rwasm_runtime)
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<Fr>,
        ) -> Result<(), Error> {
            let (selector, rwasm_config) = config;
            layouter.assign_region(
                || "test",
                |mut region| {
                    for offset in 0..self.rwasm_binary.len() {
                        selector.enable(&mut region, offset);
                    }
                    rwasm_config.assign(&mut region, &self.rwasm_binary)?;
                    Ok(())
                },
            )?;
            rwasm_config.load(&mut layouter, &self.rwasm_binary)?;
            Ok(())
        }
    }

    #[test]
    fn test_basic_circuit() {
        let circuit = TestCircuit {
            rwasm_binary: "hello, world".to_string().into_bytes(),
        };
        let k = 8;
        let is_ok = true;
        let prover = MockProver::<Fr>::run(k, &circuit, vec![]).unwrap();
        if is_ok {
            prover.assert_satisfied();
        } else {
            assert!(prover.verify().is_err());
        }
    }
}

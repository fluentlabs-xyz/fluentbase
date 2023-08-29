use crate::{
    pi_circuit::PublicInputCircuitConfig,
    poseidon_circuit::{PoseidonCircuitConfig, PoseidonTable},
    runtime_circuit::RuntimeCircuitConfig,
    rwasm_circuit::RwasmCircuitConfig,
    unrolled_bytecode::UnrolledBytecode,
    util::Field,
};
use fluentbase_rwasm::engine::Tracer;
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner},
    halo2curves::bn256::Fr,
    plonk::{Circuit, ConstraintSystem, Error},
};

#[derive(Clone)]
pub struct FluentbaseCircuitConfig<F: Field> {
    poseidon_circuit_config: PoseidonCircuitConfig<F>,
    rwasm_circuit_config: RwasmCircuitConfig<F>,
    runtime_circuit_config: RuntimeCircuitConfig<F>,
    pi_circuit_config: PublicInputCircuitConfig<F>,
}

impl<F: Field> FluentbaseCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        // init shared poseidon table
        let poseidon_table = PoseidonTable::configure(cs);
        // init poseidon and rwasm circuits
        let poseidon_circuit_config = PoseidonCircuitConfig::configure(cs, poseidon_table.clone());
        let rwasm_circuit_config = RwasmCircuitConfig::configure(cs, &poseidon_table);
        let runtime_circuit_config = RuntimeCircuitConfig::configure(cs, &rwasm_circuit_config);
        let pi_circuit_config = PublicInputCircuitConfig::configure(cs, &poseidon_table);
        Self {
            poseidon_circuit_config,
            rwasm_circuit_config,
            runtime_circuit_config,
            pi_circuit_config,
        }
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledBytecode<F>,
        tracer: Option<&Tracer>,
        hash_value: F,
    ) -> Result<(), Error> {
        self.poseidon_circuit_config
            .assign_bytecode(layouter, bytecode)?;
        self.rwasm_circuit_config.assign(layouter, bytecode)?;
        self.rwasm_circuit_config.load(layouter)?;
        if let Some(tracer) = tracer {
            self.runtime_circuit_config.assign(layouter, tracer)?;
        }
        self.pi_circuit_config.expose_public(layouter, hash_value)?;
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct FluentbaseCircuit<'tracer, F: Field> {
    pub(crate) bytecode: UnrolledBytecode<F>,
    pub(crate) tracer: Option<&'tracer Tracer>,
    pub(crate) hash_value: F,
}

impl<'tracer, F: Field> Circuit<F> for FluentbaseCircuit<'tracer, F> {
    type Config = FluentbaseCircuitConfig<F>;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(cs: &mut ConstraintSystem<F>) -> Self::Config {
        let rwasm_runtime = FluentbaseCircuitConfig::configure(cs);
        rwasm_runtime
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<F>,
    ) -> Result<(), Error> {
        config.assign(&mut layouter, &self.bytecode, self.tracer, self.hash_value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_rwasm::instruction_set;
    use halo2_proofs::dev::MockProver;

    fn test_ok<I: Into<Vec<u8>>>(bytecode: I) {
        let bytecode: Vec<u8> = bytecode.into();
        let hash_value = Fr::zero();
        let circuit = FluentbaseCircuit {
            bytecode: UnrolledBytecode::new(bytecode.as_slice()),
            tracer: Default::default(),
            hash_value,
        };
        let k = 10;
        let prover = MockProver::<Fr>::run(k, &circuit, vec![vec![hash_value]]).unwrap();
        prover.assert_satisfied();
    }

    #[test]
    fn test_add_three_numbers() {
        // test for odd instruction number
        test_ok(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
        ));
        // test for even instruction number
        test_ok(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
            .op_i32_const(3)
            .op_i32_add()
            .op_drop()
        ));
    }

    #[test]
    fn test_illegal_opcode() {
        let bytecode = vec![0xf3];
        test_ok(bytecode);
    }

    #[test]
    fn test_need_more() {
        // 63 is `i32.const` code, it should has 8 bytes after
        let bytecode = vec![63];
        test_ok(bytecode);
        // 63 is `i32.const` code, it should has 8 bytes after
        let bytecode = vec![63, 0x00, 0x00, 0x00];
        test_ok(bytecode);
        // 63 is `i32.const` code, it should has 8 bytes after
        let bytecode = vec![63, 0x00, 0x00, 0x00, 0x00];
        test_ok(bytecode);
    }
}

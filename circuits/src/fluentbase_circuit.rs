use crate::{
    exec_step::ExecSteps,
    fixed_table::FixedTable,
    pi_circuit::PublicInputCircuitConfig,
    poseidon_circuit::{PoseidonCircuitConfig, PoseidonTable},
    range_check::RangeCheckConfig,
    runtime_circuit::RuntimeCircuitConfig,
    rwasm_circuit::RwasmCircuitConfig,
    state_circuit::StateCircuitConfig,
    util::Field,
    witness::UnrolledInstructionSet,
};
use fluentbase_runtime::ExecutionResult;
use fluentbase_rwasm::engine::Tracer;
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner},
    plonk::{Circuit, ConstraintSystem, Error},
};

#[derive(Clone)]
pub struct FluentbaseCircuitConfig<F: Field> {
    // sub-circuits
    poseidon_circuit_config: PoseidonCircuitConfig<F>,
    rwasm_circuit_config: RwasmCircuitConfig<F>,
    runtime_circuit_config: RuntimeCircuitConfig<F>,
    pi_circuit_config: PublicInputCircuitConfig<F>,
    state_circuit_config: StateCircuitConfig<F>,
    // tables
    poseidon_table: PoseidonTable,
    range_check_table: RangeCheckConfig<F>,
    fixed_table: FixedTable<F>,
}

impl<F: Field> FluentbaseCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        // init shared poseidon table
        let poseidon_table = PoseidonTable::configure(cs);
        let range_check_table = RangeCheckConfig::configure(cs);
        let fixed_table = FixedTable::configure(cs);
        // init poseidon and rwasm circuits
        let poseidon_circuit_config = PoseidonCircuitConfig::configure(cs, &poseidon_table);
        let rwasm_circuit_config = RwasmCircuitConfig::configure(cs, &poseidon_table);
        let state_circuit_config = StateCircuitConfig::configure(cs, &range_check_table);
        let pi_circuit_config = PublicInputCircuitConfig::configure(cs, &poseidon_table);
        let runtime_circuit_config = RuntimeCircuitConfig::configure(
            cs,
            &rwasm_circuit_config,
            &state_circuit_config,
            &range_check_table,
            &fixed_table,
            &pi_circuit_config,
        );
        Self {
            poseidon_circuit_config,
            rwasm_circuit_config,
            runtime_circuit_config,
            pi_circuit_config,
            state_circuit_config,
            poseidon_table,
            range_check_table,
            fixed_table,
        }
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledInstructionSet<F>,
        tracer: Option<&Tracer>,
    ) -> Result<(), Error> {
        // load lookup tables
        self.range_check_table.load(layouter)?;
        self.fixed_table.load(layouter)?;
        // assign bytecode
        self.poseidon_circuit_config
            .assign_bytecode(layouter, bytecode)?;
        self.rwasm_circuit_config.assign(layouter, bytecode)?;
        self.rwasm_circuit_config.load(layouter)?;
        if let Some(tracer) = tracer {
            // TODO: "normal error conversion here"
            let exec_steps = ExecSteps::from_tracer(tracer).unwrap();
            self.runtime_circuit_config.assign(layouter, &exec_steps)?;
            self.state_circuit_config.assign(layouter, &exec_steps)?;
        }
        self.pi_circuit_config.expose_public(layouter)?;
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct FluentbaseCircuit<'tracer, F: Field> {
    pub(crate) bytecode: UnrolledInstructionSet<F>,
    pub(crate) tracer: Option<&'tracer Tracer>,
    pub(crate) input: Vec<u8>,
    pub(crate) output: Vec<u8>,
    pub(crate) exit_code: i32,
}

impl<'tracer, F: Field> FluentbaseCircuit<'tracer, F> {
    pub fn from_execution_result(execution_result: &'tracer ExecutionResult) -> Self {
        Self {
            bytecode: UnrolledInstructionSet::new(execution_result.bytecode().as_slice()),
            tracer: Some(execution_result.tracer()),
            input: execution_result.data().input().clone(),
            output: execution_result.data().output().clone(),
            exit_code: execution_result.data().exit_code(),
        }
    }
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
        config.assign(&mut layouter, &self.bytecode, self.tracer)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_rwasm::instruction_set;
    use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

    fn test_ok<I: Into<Vec<u8>>>(bytecode: I) {
        let bytecode: Vec<u8> = bytecode.into();
        let hash_value = Fr::zero();
        let circuit = FluentbaseCircuit {
            bytecode: UnrolledInstructionSet::new(bytecode.as_slice()),
            tracer: None,
            input: vec![],
            output: vec![],
            exit_code: 0,
        };
        let k = 17;
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

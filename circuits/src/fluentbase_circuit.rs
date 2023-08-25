use crate::{
    gadgets::poseidon::PoseidonTable,
    poseidon_circuit::PoseidonCircuitConfig,
    rwasm_circuit::RwasmCircuitConfig,
    unrolled_bytecode::UnrolledBytecode,
    util::Field,
};
use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner},
    halo2curves::bn256::Fr,
    plonk::{Circuit, ConstraintSystem, Error},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct FluentbaseCircuitConfig<F: Field> {
    poseidon_circuit_config: PoseidonCircuitConfig<F>,
    rwasm_circuit_config: RwasmCircuitConfig<F>,
}

impl<F: Field> FluentbaseCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        // init shared poseidon table
        let poseidon_table = PoseidonTable::configure(cs);
        // init poseidon and rwasm circuits
        let poseidon_circuit_config = PoseidonCircuitConfig::configure(cs, poseidon_table.clone());
        let rwasm_circuit_config = RwasmCircuitConfig::configure(cs, poseidon_table.clone());
        Self {
            poseidon_circuit_config,
            rwasm_circuit_config,
        }
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledBytecode<F>,
    ) -> Result<(), Error> {
        self.poseidon_circuit_config
            .assign_bytecode(layouter, bytecode)?;
        self.rwasm_circuit_config.assign(layouter, bytecode)?;
        self.rwasm_circuit_config.load(layouter)?;
        Ok(())
    }
}

#[derive(Clone, Default, Debug)]
struct FluentbaseCircuit<F: Field> {
    bytecode: UnrolledBytecode<F>,
    _pd: PhantomData<F>,
}

impl<F: Field> Circuit<F> for FluentbaseCircuit<F> {
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
        config.assign(&mut layouter, &self.bytecode)?;
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
        let circuit = FluentbaseCircuit {
            bytecode: UnrolledBytecode::new(bytecode.as_slice()),
            _pd: Default::default(),
        };
        let k = 10;
        let prover = MockProver::<Fr>::run(k, &circuit, vec![]).unwrap();
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
    #[ignore]
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

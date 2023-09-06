use crate::fluentbase_circuit::FluentbaseCircuit;
use fluentbase_runtime::Runtime;
use fluentbase_rwasm::rwasm::InstructionSet;
use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

pub(crate) fn test_ok(mut bytecode: InstructionSet) {
    bytecode.finalize(true);
    let bytecode: Vec<u8> = bytecode.into();
    let execution_result = Runtime::run(bytecode.as_slice(), &[]).unwrap();
    let circuit = FluentbaseCircuit::from_execution_result(&execution_result);
    let k = 14;
    let prover = MockProver::<Fr>::run(k, &circuit, vec![vec![Fr::zero()]]).unwrap();
    prover.assert_satisfied();
}

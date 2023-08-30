use crate::{fluentbase_circuit::FluentbaseCircuit, unrolled_bytecode::UnrolledBytecode};
use fluentbase_runtime::Runtime;
use fluentbase_rwasm::rwasm::InstructionSet;
use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

pub(crate) fn test_ok(mut bytecode: InstructionSet) {
    bytecode.finalize(true);
    let bytecode: Vec<u8> = bytecode.into();
    let execution_result = Runtime::run(bytecode.as_slice(), &[]).unwrap();
    let circuit = FluentbaseCircuit {
        bytecode: UnrolledBytecode::new(bytecode.as_slice()),
        tracer: Some(execution_result.tracer()),
        input_hash: Fr::zero(),
    };
    let k = 17;
    let prover = MockProver::<Fr>::run(k, &circuit, vec![vec![Fr::zero()]]).unwrap();
    prover.assert_satisfied();
}
